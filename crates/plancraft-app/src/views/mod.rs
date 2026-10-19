//! Main views.

pub mod calendar;
pub mod gantt;
pub mod gantt_chart;
pub mod milestones;
pub mod resource_sheet;
pub mod resource_usage;
pub mod task_usage;
pub mod team_planner;
pub mod timeline;
pub mod timesheet;
pub mod workload;

use std::collections::HashSet;

use plancraft_core::prelude::*;

use crate::columns;
use crate::state::{Selection, TaskFilter, TaskGroup, UiState};

/// The task shown on display row `row`, where `rows` maps rows to task indices.
pub fn task_at<'a>(p: &'a Project, rows: &[usize], row: usize) -> Option<&'a Task> {
    rows.get(row).and_then(|&i| p.tasks.get(i))
}

pub fn matches_filter(p: &Project, t: &Task, f: TaskFilter, value: f64) -> bool {
    let leaf = !t.sched.is_summary;
    let started = t.percent_complete > 0 || t.actual_start.is_some();
    match f {
        TaskFilter::InProgress => leaf && started && !t.is_complete(),
        TaskFilter::Unstarted => leaf && !started,
        TaskFilter::StartingSoon => {
            let at = plancraft_core::report::status_moment(p);
            let days = if value > 0.0 { value } else { 7.0 };
            let until = at + chrono::Duration::minutes((days * 1440.0) as i64);
            leaf && !started && t.sched.start.is_some_and(|s| s <= until)
        }
        TaskFilter::CostGreater => t.sched.cost > value,
        TaskFilter::WorkGreater => t.sched.work as f64 / 60.0 > value,
        TaskFilter::All => true,
        TaskFilter::Critical => t.sched.critical,
        TaskFilter::Incomplete => !t.is_complete(),
        TaskFilter::Completed => t.is_complete(),
        TaskFilter::Milestones => t.is_milestone(),
        TaskFilter::Late => columns::is_late(p, t),
        TaskFilter::Summary => t.sched.is_summary,
        TaskFilter::Unassigned => !t.sched.is_summary && !t.is_milestone() && p.assignments_for_task(t.id).next().is_none(),
    }
}

/// Display rows at or above this index are group headers ("Group by"), not tasks.
pub const GROUP_BASE: usize = usize::MAX / 2;

/// A group header row of a grouped task view.
#[derive(Clone, Debug, Default)]
pub struct GroupHeader {
    pub label: String,
    pub start: Option<chrono::NaiveDateTime>,
    pub finish: Option<chrono::NaiveDateTime>,
    pub cost: f64,
    pub work: Minutes,
    pub percent: f32,
    pub tasks: usize,
}

thread_local! {
    /// Headers of the last grouped `visible_rows` (row `GROUP_BASE + i` is header `i`).
    static GROUP_HEADERS: std::cell::RefCell<Vec<GroupHeader>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The group header shown on display row index `idx`, if it is one.
pub fn group_header(idx: usize) -> Option<GroupHeader> {
    let i = idx.checked_sub(GROUP_BASE)?;
    GROUP_HEADERS.with(|h| h.borrow().get(i).cloned())
}

/// Task indices shown in task views, honoring collapsed summaries, the filter and search text.
/// With "Group by", group headers (indices from `GROUP_BASE`) are interleaved.
pub fn visible_rows(p: &Project, st: &UiState) -> Vec<usize> {
    if st.group != TaskGroup::None {
        return grouped_rows(p, st);
    }
    visible_rows_ungrouped(p, st)
}

/// Group keys of a task: (order, label) for each group it belongs to.
fn group_keys(p: &Project, t: &Task, g: TaskGroup) -> Vec<(i64, String)> {
    let field = |name: &str, value: String| format!("{}: {}", crate::tr_dyn(name), value);
    let yes_no = |b: bool| if b { crate::tr("Yes").to_string() } else { crate::tr("No").to_string() };
    match g {
        TaskGroup::None => vec![(0, String::new())],
        TaskGroup::CompleteIncomplete => {
            let pct = t.percent_complete;
            let (o, l) = match pct {
                0 => (0, "0%"),
                100 => (2, "100%"),
                _ => (1, "1% – 99%"),
            };
            vec![(o, field("% Complete", l.to_string()))]
        }
        TaskGroup::ConstraintType => {
            let o = ConstraintKind::ALL.iter().position(|k| *k == t.constraint.kind).unwrap_or(0) as i64;
            vec![(o, field("Constraint Type", t.constraint.kind.label().to_string()))]
        }
        TaskGroup::Critical => vec![(i64::from(t.sched.critical), field("Critical task", yes_no(t.sched.critical)))],
        TaskGroup::Duration => {
            vec![(t.sched.duration, field("Duration", p.format_duration(t.sched.duration, t.estimated)))]
        }
        TaskGroup::Milestones => vec![(i64::from(t.is_milestone()), field("Milestone", yes_no(t.is_milestone())))],
        TaskGroup::Priority => vec![(i64::from(t.priority), field("Priority", t.priority.to_string()))],
        TaskGroup::Resource => {
            let mut names: Vec<String> =
                p.assignments_for_task(t.id).filter_map(|a| p.resource(a.resource)).map(|r| r.name.clone()).collect();
            names.sort();
            names.dedup();
            if names.is_empty() {
                vec![(0, field("Resource Names", crate::tr("(none)").to_string()))]
            } else {
                names.into_iter().map(|n| (1, field("Resource Names", n))).collect()
            }
        }
        TaskGroup::Status => {
            let at = plancraft_core::report::status_moment(p);
            let (o, l) = if t.is_complete() {
                (0, "Complete")
            } else if columns::is_late(p, t) {
                (2, "Late")
            } else if t.sched.start.is_some_and(|s| s > at) && t.percent_complete == 0 {
                (3, "Future Task")
            } else {
                (1, "On Schedule")
            };
            vec![(o, field("Status", crate::tr(l).to_string()))]
        }
    }
}

/// Rows of a grouped view: subtasks (no summaries) under one header per group value, like
/// Microsoft Project's "Group by". A task assigned to several resources appears in each group.
fn grouped_rows(p: &Project, st: &UiState) -> Vec<usize> {
    let search = st.search.trim().to_lowercase();
    let value = plancraft_core::i18n::parse_f64(&st.filter_value).unwrap_or(0.0);
    let mut groups: std::collections::BTreeMap<(i64, String), Vec<usize>> = std::collections::BTreeMap::new();
    for i in display_order(p, st.sort) {
        let Some(t) = p.tasks.get(i) else { continue };
        if p.has_children(i) || !matches_filter(p, t, st.filter, value) {
            continue;
        }
        if !search.is_empty() && !t.name.to_lowercase().contains(&search) {
            continue;
        }
        for key in group_keys(p, t, st.group) {
            groups.entry(key).or_default().push(i);
        }
    }
    let mut headers = Vec::with_capacity(groups.len());
    let mut rows = Vec::new();
    for ((_, label), members) in groups {
        let mut h = GroupHeader { label: label.clone(), tasks: members.len(), ..Default::default() };
        let (mut pw, mut pd) = (0.0f64, 0.0f64);
        for &i in &members {
            let Some(t) = p.tasks.get(i) else { continue };
            h.start = match (h.start, t.sched.start) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
            h.finish = match (h.finish, t.sched.finish) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (a, b) => a.or(b),
            };
            h.cost += t.sched.cost;
            h.work += t.sched.work;
            let w = t.sched.duration.max(1) as f64;
            pw += w * f64::from(t.sched.percent_complete);
            pd += w;
        }
        h.percent = if pd > 0.0 { (pw / pd) as f32 } else { 0.0 };
        rows.push(GROUP_BASE + headers.len());
        headers.push(h);
        if !st.collapsed_groups.contains(&label) {
            rows.extend(members);
        }
    }
    GROUP_HEADERS.with(|g| *g.borrow_mut() = headers);
    rows
}

/// Task indices shown in task views without grouping (Task Usage, reports).
pub fn visible_rows_ungrouped(p: &Project, st: &UiState) -> Vec<usize> {
    let search = st.search.trim().to_lowercase();
    let filtering = st.filter != TaskFilter::All || !search.is_empty();
    let value = plancraft_core::i18n::parse_f64(&st.filter_value).unwrap_or(0.0);
    let mut keep: Vec<bool> = vec![!filtering; p.tasks.len()];
    if filtering {
        for (i, t) in p.tasks.iter().enumerate() {
            let ok = matches_filter(p, t, st.filter, value) && (search.is_empty() || t.name.to_lowercase().contains(&search));
            if !ok {
                continue;
            }
            // Keep summary rows above matches as context, like "show related summary rows".
            let mut a = Some(i);
            while let Some(ai) = a {
                if let Some(k) = keep.get_mut(ai) {
                    *k = true;
                }
                a = p.parent_index(ai);
            }
        }
    }
    let mut rows = Vec::with_capacity(p.tasks.len());
    let mut skip_deeper: Option<u16> = None;
    for i in display_order(p, st.sort) {
        let Some(t) = p.tasks.get(i) else { continue };
        if st.show_level > 0 && t.outline_level > st.show_level {
            continue;
        }
        if let Some(lvl) = skip_deeper {
            if t.outline_level > lvl {
                continue;
            }
            skip_deeper = None;
        }
        if !keep.get(i).copied().unwrap_or(false) {
            continue;
        }
        rows.push(i);
        if st.collapsed.contains(&t.id) && p.has_children(i) {
            skip_deeper = Some(t.outline_level);
        }
    }
    rows
}

/// Task indices in display order: outline order, or siblings sorted by `key` (keeping each
/// summary's subtasks under it, like Microsoft Project's "Keep outline structure").
pub fn display_order(p: &Project, key: plancraft_core::extras::SortKey) -> Vec<usize> {
    use plancraft_core::extras::SortKey;
    if key == SortKey::Id {
        return (0..p.tasks.len()).collect();
    }
    // Children of each row (None = top level).
    let mut children: std::collections::HashMap<Option<usize>, Vec<usize>> = std::collections::HashMap::new();
    for i in 0..p.tasks.len() {
        children.entry(p.parent_index(i)).or_default().push(i);
    }
    let cmp = |a: &usize, b: &usize| -> std::cmp::Ordering {
        let (Some(x), Some(y)) = (p.tasks.get(*a), p.tasks.get(*b)) else { return std::cmp::Ordering::Equal };
        let o = match key {
            SortKey::Cost => y.sched.cost.total_cmp(&x.sched.cost),
            SortKey::Name => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
            SortKey::Start => x.sched.start.cmp(&y.sched.start),
            SortKey::Finish => x.sched.finish.cmp(&y.sched.finish),
            SortKey::Duration => y.sched.duration.cmp(&x.sched.duration),
            SortKey::Work => y.sched.work.cmp(&x.sched.work),
            SortKey::Priority => y.priority.cmp(&x.priority),
            SortKey::Id => std::cmp::Ordering::Equal,
        };
        o.then(a.cmp(b))
    };
    for v in children.values_mut() {
        v.sort_by(cmp);
    }
    let mut out = Vec::with_capacity(p.tasks.len());
    let mut stack: Vec<usize> = children.get(&None).map(|v| v.iter().rev().copied().collect()).unwrap_or_default();
    while let Some(i) = stack.pop() {
        out.push(i);
        if let Some(c) = children.get(&Some(i)) {
            stack.extend(c.iter().rev().copied());
        }
    }
    out
}

/// Applies a click on `row` to the task selection.
pub fn select_row(sel: &mut Selection, p: &Project, rows: &[usize], row: usize, shift: bool, ctrl: bool) {
    let Some(id) = task_at(p, rows, row).map(|t| t.id) else { return };
    if shift && let Some(anchor) = sel.anchor {
        let a = rows.iter().position(|&i| p.tasks.get(i).is_some_and(|t| t.id == anchor)).unwrap_or(row);
        let (lo, hi) = if a <= row { (a, row) } else { (row, a) };
        if !ctrl {
            sel.tasks.clear();
        }
        for i in rows.iter().take(hi + 1).skip(lo) {
            if let Some(t) = p.tasks.get(*i) {
                sel.tasks.insert(t.id);
            }
        }
    } else if ctrl {
        if !sel.tasks.remove(&id) {
            sel.tasks.insert(id);
        }
        sel.anchor = Some(id);
    } else {
        sel.select_task(id);
    }
}

/// Selected task ids in outline order.
pub fn selected_in_order(p: &Project, sel: &Selection) -> Vec<TaskId> {
    p.tasks.iter().filter(|t| sel.tasks.contains(&t.id)).map(|t| t.id).collect()
}

/// Task ids that have an assignment to an overallocated resource.
pub fn overallocated_tasks(p: &Project, over: &HashSet<ResourceId>) -> HashSet<TaskId> {
    if over.is_empty() {
        return HashSet::new();
    }
    p.assignments.iter().filter(|a| over.contains(&a.resource)).map(|a| a.task).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::columns::{TaskCol, commit};

    fn demo() -> Project {
        plancraft_core::sample::demo_project(chrono::NaiveDate::from_ymd_opt(2026, 10, 19).unwrap())
    }

    #[test]
    fn collapse_and_filter_rows() {
        let p = demo();
        let mut st = UiState::default();
        assert_eq!(visible_rows(&p, &st).len(), p.tasks.len());
        let planning = p.tasks[0].id;
        st.collapsed.insert(planning);
        assert_eq!(visible_rows(&p, &st).len(), p.tasks.len() - 3);
        st.collapsed.clear();
        st.filter = TaskFilter::Milestones;
        let rows = visible_rows(&p, &st);
        // 4 milestones plus their 4 summary parents.
        assert_eq!(rows.len(), 8);
        assert!(rows.iter().all(|&i| p.tasks[i].is_milestone() || p.tasks[i].sched.is_summary));
    }

    #[test]
    fn group_by_adds_headers() {
        let p = demo();
        let mut st = UiState { group: TaskGroup::Critical, ..Default::default() };
        let rows = visible_rows(&p, &st);
        let headers: Vec<GroupHeader> = rows.iter().filter_map(|&r| group_header(r)).collect();
        assert!(!headers.is_empty() && headers.len() <= 2);
        let leaves = (0..p.tasks.len()).filter(|&i| !p.has_children(i)).count();
        assert_eq!(rows.len(), leaves + headers.len());
        assert_eq!(headers.iter().map(|h| h.tasks).sum::<usize>(), leaves);
        // Collapsing a group hides its tasks.
        st.collapsed_groups.insert(headers[0].label.clone());
        assert_eq!(visible_rows(&p, &st).len(), leaves + headers.len() - headers[0].tasks);
        // Grouping by resource can repeat a task under each of its resources.
        st.collapsed_groups.clear();
        st.group = TaskGroup::Resource;
        assert!(visible_rows(&p, &st).iter().any(|&r| r >= GROUP_BASE));
        for g in TaskGroup::ALL {
            st.group = g;
            let _ = visible_rows(&p, &st);
        }
    }

    #[test]
    fn column_commits_follow_ms_project_rules() {
        let mut p = demo();
        let id = p.tasks[1].id;
        commit(TaskCol::Duration, &mut p, id, "4d").unwrap();
        assert_eq!(p.task(id).unwrap().duration, 4 * 480);
        // Typing a start date on an auto task sets a Start No Earlier Than constraint.
        commit(TaskCol::Start, &mut p, id, "2026-11-02").unwrap();
        assert_eq!(p.task(id).unwrap().constraint.kind, ConstraintKind::Snet);
        assert!(commit(TaskCol::Duration, &mut p, id, "abc").is_err());
        assert!(commit(TaskCol::Name, &mut p, id, "  ").is_err());
        // Row 3 already depends on this task, so linking back is rejected.
        assert!(commit(TaskCol::Predecessors, &mut p, id, "3").is_err());
        let event = p.tasks[16].id;
        commit(TaskCol::Predecessors, &mut p, event, "14FS+1d").unwrap();
        assert_eq!(p.format_predecessors(event), "14FS+1 day");
    }
}
