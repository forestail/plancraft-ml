//! Reports window ("Report > Reports"): task, cost, resource and earned-value reports, with
//! export to an HTML page ("publish on the web") and to a CSV table for Excel.

use egui::{Context, RichText};
use plancraft_core::extras::{self, CashFlowRow};
use plancraft_core::i18n;
use plancraft_core::prelude::*;
use plancraft_core::workload::Granularity;
use plancraft_core::workspace::Document;

use crate::columns::{self, CellCtx, TaskCol, TaskTable};
use crate::format;
use crate::state::TaskFilter;
use crate::theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ReportKind {
    #[default]
    ProjectOverview,
    UpcomingTasks,
    UnstartedTasks,
    TasksInProgress,
    CriticalTasks,
    CashFlow,
    TaskCostOverview,
    ResourceCostOverview,
    WhoDoesWhat,
    OverallocatedResources,
    EarnedValue,
    Custom,
}

impl ReportKind {
    pub const ALL: [ReportKind; 12] = [
        ReportKind::ProjectOverview,
        ReportKind::UpcomingTasks,
        ReportKind::UnstartedTasks,
        ReportKind::TasksInProgress,
        ReportKind::CriticalTasks,
        ReportKind::CashFlow,
        ReportKind::TaskCostOverview,
        ReportKind::ResourceCostOverview,
        ReportKind::WhoDoesWhat,
        ReportKind::OverallocatedResources,
        ReportKind::EarnedValue,
        ReportKind::Custom,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ReportKind::ProjectOverview => crate::tr("Project Overview"),
            ReportKind::UpcomingTasks => crate::tr("Upcoming Tasks"),
            ReportKind::UnstartedTasks => crate::tr("Unstarted Tasks"),
            ReportKind::TasksInProgress => crate::tr("Tasks In Progress"),
            ReportKind::CriticalTasks => crate::tr("Critical Tasks"),
            ReportKind::CashFlow => crate::tr("Cash Flow"),
            ReportKind::TaskCostOverview => crate::tr("Task Cost Overview"),
            ReportKind::ResourceCostOverview => crate::tr("Resource Cost Overview"),
            ReportKind::WhoDoesWhat => crate::tr("Who Does What"),
            ReportKind::OverallocatedResources => crate::tr("Overallocated Resources"),
            ReportKind::EarnedValue => crate::tr("Earned Value Report"),
            ReportKind::Custom => crate::tr("Custom Report…"),
        }
    }
}

/// A report as a table (plus the cash-flow series for its chart).
pub struct ReportTable {
    pub title: String,
    pub headers: Vec<String>,
    pub right: Vec<bool>,
    pub rows: Vec<Vec<String>>,
    /// Bold rows (summary tasks, totals).
    pub bold: Vec<bool>,
    pub cash: Vec<CashFlowRow>,
    pub note: Option<String>,
}

impl ReportTable {
    fn new(title: &str, headers: &[(&'static str, bool)]) -> Self {
        Self {
            title: title.to_string(),
            headers: headers.iter().map(|(h, _)| crate::tr(h).to_string()).collect(),
            right: headers.iter().map(|(_, r)| *r).collect(),
            rows: Vec::new(),
            bold: Vec::new(),
            cash: Vec::new(),
            note: None,
        }
    }
    fn push(&mut self, row: Vec<String>, bold: bool) {
        self.rows.push(row);
        self.bold.push(bold);
    }
}

fn period_label(d: chrono::NaiveDate, g: Granularity) -> String {
    match g {
        Granularity::Month => format!("{} {}", i18n::month_long(chrono::Datelike::month(&d)), chrono::Datelike::year(&d)),
        _ => crate::tr_dyn(&format!("Week of {}", format::df(&d, "%b %-d, '%y"))),
    }
}

fn task_rows(p: &Project, t: &mut ReportTable, idxs: &[usize]) {
    for &i in idxs {
        let Some(task) = p.tasks.get(i) else { continue };
        t.push(
            vec![
                (i + 1).to_string(),
                task.name.clone(),
                p.format_duration(task.sched.duration, task.estimated),
                format::date(task.sched.start),
                format::date(task.sched.finish),
                format::percent(task.sched.percent_complete),
                p.format_resource_names(task.id),
            ],
            task.sched.is_summary,
        );
    }
}

const TASK_HEADERS: [(&str, bool); 7] = [
    ("ID", true),
    ("Task Name", false),
    ("Duration", true),
    ("Start", false),
    ("Finish", false),
    ("% Comp.", true),
    ("Resource Names", false),
];

// ---------------------------------------------------------------------------
// Custom report ("Report ▸ New Report ▸ Custom")
// ---------------------------------------------------------------------------

/// Sort order of a custom report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CustomSort {
    #[default]
    Id,
    Name,
    Start,
    Finish,
    Duration,
    PercentComplete,
    Work,
    Cost,
    Priority,
}

impl CustomSort {
    pub const ALL: [CustomSort; 9] = [
        CustomSort::Id,
        CustomSort::Name,
        CustomSort::Start,
        CustomSort::Finish,
        CustomSort::Duration,
        CustomSort::PercentComplete,
        CustomSort::Work,
        CustomSort::Cost,
        CustomSort::Priority,
    ];
    pub fn label(self) -> &'static str {
        match self {
            CustomSort::Id => crate::tr("ID"),
            CustomSort::Name => crate::tr("Name"),
            CustomSort::Start => crate::tr("Start"),
            CustomSort::Finish => crate::tr("Finish"),
            CustomSort::Duration => crate::tr("Duration"),
            CustomSort::PercentComplete => crate::tr("% Complete"),
            CustomSort::Work => crate::tr("Work"),
            CustomSort::Cost => crate::tr("Cost"),
            CustomSort::Priority => crate::tr("Priority"),
        }
    }
}

/// What a custom report shows: a task table, filtered and sorted, optionally with the
/// assignments (resource, work, cost) under each task.
#[derive(Clone, Debug)]
pub struct CustomSpec {
    pub title: String,
    pub table: TaskTable,
    pub filter: TaskFilter,
    pub filter_value: String,
    pub sort: CustomSort,
    pub descending: bool,
    pub summaries: bool,
    pub assignments: bool,
}

impl Default for CustomSpec {
    fn default() -> Self {
        Self {
            title: crate::tr("Custom Report").to_string(),
            table: TaskTable::Entry,
            filter: TaskFilter::All,
            filter_value: String::new(),
            sort: CustomSort::Id,
            descending: false,
            summaries: true,
            assignments: false,
        }
    }
}

thread_local! {
    /// The custom report last set up (kept while the program runs).
    static LAST_SPEC: std::cell::RefCell<Option<CustomSpec>> = const { std::cell::RefCell::new(None) };
}

/// Builds a custom report.
pub fn build_custom(p: &Project, spec: &CustomSpec) -> ReportTable {
    let mut cols: Vec<TaskCol> = spec.table.columns().into_iter().filter(|c| !matches!(c, TaskCol::Indicators | TaskCol::Mode)).collect();
    if spec.assignments {
        if !cols.contains(&TaskCol::Resources) && !cols.is_empty() {
            let at = cols.iter().position(|c| *c == TaskCol::Name).map(|i| i + 1).unwrap_or(cols.len());
            cols.insert(at.min(cols.len()), TaskCol::Resources);
        }
        for c in [TaskCol::Work, TaskCol::Cost] {
            if !cols.contains(&c) {
                cols.push(c);
            }
        }
    }
    let defs: Vec<_> = cols.iter().map(|c| c.def(p)).collect();
    let mut t = ReportTable {
        title: if spec.title.trim().is_empty() { crate::tr("Custom Report").to_string() } else { spec.title.trim().to_string() },
        headers: defs.iter().map(|d| d.title.clone()).collect(),
        right: defs.iter().map(|d| d.align == egui::Align::RIGHT).collect(),
        rows: Vec::new(),
        bold: Vec::new(),
        cash: Vec::new(),
        note: None,
    };
    let value = i18n::parse_f64(&spec.filter_value).unwrap_or(0.0);
    let mut idxs: Vec<usize> = (0..p.tasks.len())
        .filter(|&i| p.tasks.get(i).is_some_and(|task| crate::views::matches_filter(p, task, spec.filter, value)))
        .filter(|&i| spec.summaries || !p.has_children(i))
        .collect();
    let key = |i: &usize| p.tasks.get(*i);
    idxs.sort_by(|a, b| {
        let (Some(x), Some(y)) = (key(a), key(b)) else { return std::cmp::Ordering::Equal };
        let o = match spec.sort {
            CustomSort::Id => a.cmp(b),
            CustomSort::Name => x.name.to_lowercase().cmp(&y.name.to_lowercase()),
            CustomSort::Start => x.sched.start.cmp(&y.sched.start),
            CustomSort::Finish => x.sched.finish.cmp(&y.sched.finish),
            CustomSort::Duration => x.sched.duration.cmp(&y.sched.duration),
            CustomSort::PercentComplete => x.sched.percent_complete.total_cmp(&y.sched.percent_complete),
            CustomSort::Work => x.sched.work.cmp(&y.sched.work),
            CustomSort::Cost => x.sched.cost.total_cmp(&y.sched.cost),
            CustomSort::Priority => x.priority.cmp(&y.priority),
        };
        let o = if spec.descending { o.reverse() } else { o };
        o.then(a.cmp(b))
    });
    let no_over = |_: TaskId| false;
    let ctx = CellCtx { overallocated: &no_over };
    let (mut work_total, mut cost_total) = (0, 0.0);
    for i in idxs {
        let Some(task) = p.tasks.get(i) else { continue };
        let summary = p.has_children(i);
        t.push(cols.iter().map(|c| columns::cell(*c, p, i, &ctx).text).collect(), summary);
        if !summary {
            work_total += task.sched.work;
            cost_total += task.sched.cost;
        }
        if spec.assignments && !summary {
            for a in p.assignments_for_task(task.id) {
                let Some(r) = p.resource(a.resource) else { continue };
                let cost: f64 = extras::assignment_daily_cost(p, a).values().sum();
                let row = cols
                    .iter()
                    .map(|c| match c {
                        TaskCol::Name => format!("    ↳ {}", r.name),
                        TaskCol::Resources => format!("{} ({})", r.name, format::units(a.units)),
                        TaskCol::Work => {
                            if r.kind == ResourceType::Work {
                                plancraft_core::duration::format_work(a.work)
                            } else {
                                String::new()
                            }
                        }
                        TaskCol::Cost => format::money(p, cost),
                        TaskCol::Start => format::date(task.sched.start),
                        TaskCol::Finish => format::date(task.sched.finish),
                        _ => String::new(),
                    })
                    .collect();
                t.push(row, false);
            }
        }
    }
    if !t.rows.is_empty() {
        let row = cols
            .iter()
            .enumerate()
            .map(|(k, c)| match c {
                TaskCol::Work => plancraft_core::duration::format_work(work_total),
                TaskCol::Cost => format::money(p, cost_total),
                _ if k == 0 => crate::tr("Total").to_string(),
                _ => String::new(),
            })
            .collect();
        t.push(row, true);
    }
    let mut note = vec![crate::tr_dyn(&format!("Filter: {}", spec.filter.label()))];
    note.push(crate::tr_dyn(&format!("Sorted by: {}", spec.sort.label())));
    t.note = Some(note.join(" · "));
    t
}

/// The custom report set-up panel.
fn custom_editor(ui: &mut egui::Ui, spec: &mut CustomSpec) {
    egui::CollapsingHeader::new(RichText::new(crate::tr("Report definition")).strong()).default_open(true).show(ui, |ui| {
        egui::Grid::new("custom_rep").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(crate::tr("Name:"));
            ui.add(egui::TextEdit::singleline(&mut spec.title).desired_width(220.0));
            ui.label(crate::tr("Table:"));
            egui::ComboBox::from_id_salt("cr_table").width(180.0).selected_text(spec.table.label()).show_ui(ui, |ui| {
                for tb in TaskTable::ALL {
                    ui.selectable_value(&mut spec.table, tb, tb.label());
                }
            });
            ui.end_row();
            ui.label(crate::tr("Filter:"));
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("cr_filter").width(200.0).selected_text(spec.filter.label()).show_ui(ui, |ui| {
                    for f in TaskFilter::ALL {
                        ui.selectable_value(&mut spec.filter, f, f.label());
                    }
                });
                if spec.filter.needs_value() {
                    ui.add(egui::TextEdit::singleline(&mut spec.filter_value).desired_width(60.0).hint_text("0"));
                }
            });
            ui.label(crate::tr("Sort by:"));
            ui.horizontal(|ui| {
                egui::ComboBox::from_id_salt("cr_sort").width(140.0).selected_text(spec.sort.label()).show_ui(ui, |ui| {
                    for k in CustomSort::ALL {
                        ui.selectable_value(&mut spec.sort, k, k.label());
                    }
                });
                ui.checkbox(&mut spec.descending, crate::tr("Descending"));
            });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut spec.summaries, crate::tr("Show summary tasks"));
            ui.label("");
            ui.checkbox(&mut spec.assignments, crate::tr("Details: assignments (work and cost)"));
            ui.end_row();
        });
    });
}

/// Builds the data of a report.
pub fn build(p: &Project, kind: ReportKind, g: Granularity, days: i64) -> ReportTable {
    let money = |v: f64| format::money(p, v);
    match kind {
        ReportKind::Custom => build_custom(p, &CustomSpec::default()),
        ReportKind::ProjectOverview => {
            let s = plancraft_core::report::project_stats(p);
            let mut t = ReportTable::new(kind.label(), &[("Name", false), ("Total", true)]);
            let rows: Vec<(&str, String)> = vec![
                ("Start", format::date(s.start)),
                ("Finish", format::date(s.finish)),
                ("Duration", p.format_duration(s.duration, false)),
                ("Work", plancraft_core::duration::format_work(s.work)),
                ("Cost", money(s.cost)),
                ("Baseline cost", money(s.baseline_cost)),
                ("Percent complete", format::percent(s.percent_complete)),
                ("Tasks", s.tasks.to_string()),
                ("Completed", s.completed_tasks.to_string()),
                ("In progress", s.in_progress_tasks.to_string()),
                ("Late Tasks", s.late_tasks.to_string()),
                ("Critical Tasks", s.critical_tasks.to_string()),
                ("Milestones", s.milestones.to_string()),
                ("Resources", s.resources.to_string()),
                ("Overallocated Resources", s.overallocated_resources.to_string()),
            ];
            for (k, v) in rows {
                t.push(vec![crate::tr(k).to_string(), v], false);
            }
            t
        }
        ReportKind::UpcomingTasks => {
            let mut t = ReportTable::new(kind.label(), &TASK_HEADERS);
            task_rows(p, &mut t, &extras::upcoming_tasks(p, days));
            t.note = Some(crate::tr_dyn(&format!("Status date: {}", format::date(Some(plancraft_core::report::status_moment(p))))));
            t
        }
        ReportKind::UnstartedTasks => {
            let mut t = ReportTable::new(kind.label(), &TASK_HEADERS);
            task_rows(p, &mut t, &extras::unstarted_tasks(p));
            t
        }
        ReportKind::TasksInProgress => {
            let mut t = ReportTable::new(kind.label(), &TASK_HEADERS);
            task_rows(p, &mut t, &extras::tasks_in_progress(p));
            t
        }
        ReportKind::CriticalTasks => {
            let mut t = ReportTable::new(kind.label(), &TASK_HEADERS);
            let idx: Vec<usize> =
                p.tasks.iter().enumerate().filter(|(_, t)| t.sched.critical && !t.sched.is_summary).map(|(i, _)| i).collect();
            task_rows(p, &mut t, &idx);
            t
        }
        ReportKind::CashFlow => {
            let mut t = ReportTable::new(kind.label(), &[("Period:", false), ("Period Cost", true), ("Cumulative Cost", true)]);
            t.cash = extras::cash_flow(p, g);
            for r in &t.cash.clone() {
                t.push(vec![period_label(r.period, g), money(r.cost), money(r.cumulative)], false);
            }
            let total = t.cash.last().map(|r| r.cumulative).unwrap_or(0.0);
            t.push(vec![crate::tr("Total").to_string(), money(total), String::new()], true);
            t
        }
        ReportKind::TaskCostOverview => {
            let mut t = ReportTable::new(
                kind.label(),
                &[
                    ("ID", true),
                    ("Task Name", false),
                    ("Fixed Cost", true),
                    ("Fixed Cost Accrual", false),
                    ("Total Cost", true),
                    ("Baseline", true),
                    ("Cost Var.", true),
                    ("Actual Cost", true),
                    ("Remaining Cost", true),
                ],
            );
            for (i, task) in p.tasks.iter().enumerate() {
                let actual = extras::task_earned_value(p, i).map(|e| e.acwp).unwrap_or_else(|| actual_cost(task));
                let base = task.baseline.map(|b| b.cost);
                t.push(
                    vec![
                        (i + 1).to_string(),
                        format!("{}{}", "  ".repeat(usize::from(task.outline_level.saturating_sub(1))), task.name),
                        money(task.fixed_cost),
                        task.fixed_cost_accrual.label().to_string(),
                        money(task.sched.cost),
                        base.map(money).unwrap_or_default(),
                        base.map(|b| money(task.sched.cost - b)).unwrap_or_default(),
                        money(actual),
                        money((task.sched.cost - actual).max(0.0)),
                    ],
                    task.sched.is_summary,
                );
            }
            t
        }
        ReportKind::ResourceCostOverview => {
            let mut t = ReportTable::new(
                kind.label(),
                &[("ID", true), ("Resource Name", false), ("Type", false), ("Work", true), ("Cost", true), ("Accrue At", false)],
            );
            let mut total = 0.0;
            for (i, r) in p.resources.iter().enumerate() {
                let c = extras::resource_cost(p, r.id);
                total += c;
                let kind = match r.kind {
                    ResourceType::Work => crate::format::work_type_label(),
                    ResourceType::Material => crate::tr("Material"),
                    ResourceType::Cost => crate::tr("Cost"),
                };
                t.push(
                    vec![
                        (i + 1).to_string(),
                        r.name.clone(),
                        kind.to_string(),
                        plancraft_core::duration::format_work(extras::resource_work(p, r.id)),
                        money(c),
                        r.accrue_at.label().to_string(),
                    ],
                    false,
                );
            }
            t.push(vec![String::new(), crate::tr("Total").to_string(), String::new(), String::new(), money(total), String::new()], true);
            t
        }
        ReportKind::WhoDoesWhat => {
            let mut t = ReportTable::new(
                kind.label(),
                &[("Resource Name", false), ("Task Name", false), ("Work", true), ("Start", false), ("Finish", false)],
            );
            for r in &p.resources {
                t.push(
                    vec![
                        r.name.clone(),
                        String::new(),
                        plancraft_core::duration::format_work(extras::resource_work(p, r.id)),
                        String::new(),
                        String::new(),
                    ],
                    true,
                );
                for a in p.assignments.iter().filter(|a| a.resource == r.id) {
                    if let Some(task) = p.task(a.task) {
                        t.push(
                            vec![
                                String::new(),
                                task.name.clone(),
                                plancraft_core::duration::format_work(a.work),
                                format::date(task.sched.start),
                                format::date(task.sched.finish),
                            ],
                            false,
                        );
                    }
                }
            }
            t
        }
        ReportKind::OverallocatedResources => {
            let mut t =
                ReportTable::new(kind.label(), &[("Resource Name", false), ("Max. Units", true), ("Overallocated", true), ("Work", true)]);
            for (rid, days) in plancraft_core::workload::overallocations(p) {
                if let Some(r) = p.resource(rid) {
                    t.push(
                        vec![
                            r.name.clone(),
                            format::units(r.max_units),
                            crate::tr_dyn(&format!("⚠ {} day(s)", days.len())),
                            plancraft_core::duration::format_work(extras::resource_work(p, rid)),
                        ],
                        false,
                    );
                }
            }
            t
        }
        ReportKind::EarnedValue => {
            let mut t = ReportTable::new(
                kind.label(),
                &[
                    ("ID", true),
                    ("Task Name", false),
                    ("BCWS", true),
                    ("BCWP", true),
                    ("ACWP", true),
                    ("SV", true),
                    ("CV", true),
                    ("SPI", true),
                    ("CPI", true),
                    ("BAC", true),
                    ("EAC", true),
                    ("VAC", true),
                ],
            );
            for (i, task) in p.tasks.iter().enumerate() {
                if let Some(ev) = extras::task_earned_value(p, i) {
                    t.push(
                        vec![
                            (i + 1).to_string(),
                            task.name.clone(),
                            money(ev.bcws),
                            money(ev.bcwp),
                            money(ev.acwp),
                            money(ev.sv),
                            money(ev.cv),
                            ratio(ev.spi),
                            ratio(ev.cpi),
                            money(ev.bac),
                            money(ev.eac),
                            money(ev.vac),
                        ],
                        task.sched.is_summary,
                    );
                }
            }
            if t.rows.is_empty() {
                t.note = Some(crate::tr("Set a baseline (Project > Set Baseline) to track variance and earned value.").to_string());
            } else {
                t.note = Some(crate::tr_dyn(&format!("Status date: {}", format::date(Some(plancraft_core::report::status_moment(p))))));
            }
            t
        }
    }
}

pub fn ratio(v: f64) -> String {
    let s = format!("{v:.2}");
    if i18n::is_ru() { s.replace('.', ",") } else { s }
}

fn actual_cost(t: &Task) -> f64 {
    if t.sched.work > 0 {
        t.sched.cost * t.sched.actual_work as f64 / t.sched.work as f64
    } else {
        t.sched.cost * t.sched.percent_complete as f64 / 100.0
    }
}

// ---------------------------------------------------------------------------
// Export
// ---------------------------------------------------------------------------

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn cash_svg(rows: &[CashFlowRow]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    let (w, h, pad) = (760.0f64, 260.0f64, 30.0f64);
    let max_cum = rows.last().map(|r| r.cumulative).unwrap_or(0.0).max(1.0);
    let max_bar = rows.iter().map(|r| r.cost).fold(0.0, f64::max).max(1.0);
    let n = rows.len() as f64;
    let bw = (w - 2.0 * pad) / n;
    let mut s = format!(r#"<svg viewBox="0 0 {w} {h}" width="100%" style="max-width:{w}px" xmlns="http://www.w3.org/2000/svg">"#);
    let mut line = String::new();
    for (i, r) in rows.iter().enumerate() {
        let x = pad + i as f64 * bw;
        let bh = (h - 2.0 * pad) * r.cost / max_bar;
        s += &format!(
            r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="#4a72d6"/>"##,
            x + bw * 0.15,
            h - pad - bh,
            bw * 0.7,
            bh
        );
        let y = h - pad - (h - 2.0 * pad) * r.cumulative / max_cum;
        line += &format!("{:.1},{:.1} ", x + bw / 2.0, y);
    }
    s += &format!(r##"<polyline points="{line}" fill="none" stroke="#e0a020" stroke-width="2.5"/>"##);
    s += &format!(r##"<line x1="{pad}" y1="{0}" x2="{1}" y2="{0}" stroke="#888"/>"##, h - pad, w - pad);
    s += "</svg>";
    s
}

pub fn to_html(p: &Project, t: &ReportTable) -> String {
    let mut s = String::from(
        "<!doctype html>\n<html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    s += &format!("<title>{} — {}</title>", html_escape(&t.title), html_escape(&p.name));
    s += "<style>body{font-family:system-ui,Segoe UI,Arial,sans-serif;margin:24px;color:#1d2433}h1{font-size:22px;margin:0 0 4px}h2{font-size:15px;color:#555;font-weight:normal;margin:0 0 16px}\
table{border-collapse:collapse;font-size:13px;min-width:50%}th{background:#1b3a8c;color:#fff;text-align:left;padding:6px 10px}td{padding:5px 10px;border-bottom:1px solid #e3e6ee}\
tr:nth-child(even) td{background:#f6f7fb}td.r,th.r{text-align:right}tr.b td{font-weight:600}.note{color:#666;margin:10px 0}footer{margin-top:20px;color:#888;font-size:12px}</style></head><body>";
    s += &format!("<h1>{}</h1><h2>{}</h2>", html_escape(&t.title), html_escape(&p.name));
    if let Some(n) = &t.note {
        s += &format!("<p class=\"note\">{}</p>", html_escape(n));
    }
    s += &cash_svg(&t.cash);
    s += "<table><thead><tr>";
    for (h, r) in t.headers.iter().zip(&t.right) {
        s += &format!("<th{}>{}</th>", if *r { " class=\"r\"" } else { "" }, html_escape(h));
    }
    s += "</tr></thead><tbody>";
    for (row, bold) in t.rows.iter().zip(&t.bold) {
        s += if *bold { "<tr class=\"b\">" } else { "<tr>" };
        for (c, r) in row.iter().zip(&t.right) {
            s += &format!("<td{}>{}</td>", if *r { " class=\"r\"" } else { "" }, html_escape(c).replace("  ", "&nbsp;&nbsp;"));
        }
        s += "</tr>";
    }
    if t.rows.is_empty() {
        s += &format!("<tr><td colspan=\"{}\">{}</td></tr>", t.headers.len().max(1), html_escape(crate::tr("No tasks match this report.")));
    }
    s += "</tbody></table>";
    let now = chrono::Local::now().naive_local();
    s += &format!(
        "<footer>{}</footer></body></html>",
        html_escape(&crate::tr_dyn(&format!("Generated by PlanCraft on {}", format::date_long(Some(now)))))
    );
    s
}

/// CSV that Excel opens directly: in Russian a BOM, `;` separators (decimal comma).
pub fn to_csv(t: &ReportTable) -> String {
    let sep = if i18n::is_ru() { ';' } else { ',' };
    let q = |v: &str| {
        let v = v.replace('\u{a0}', " ");
        if v.contains(sep) || v.contains('"') || v.contains('\n') { format!("\"{}\"", v.replace('"', "\"\"")) } else { v }
    };
    let mut s = String::new();
    if i18n::is_ru() {
        s.push('\u{feff}');
    }
    s += &t.headers.iter().map(|h| q(h)).collect::<Vec<_>>().join(&sep.to_string());
    s.push_str("\r\n");
    for row in &t.rows {
        s += &row.iter().map(|c| q(c.trim())).collect::<Vec<_>>().join(&sep.to_string());
        s.push_str("\r\n");
    }
    s
}

// ---------------------------------------------------------------------------
// Window
// ---------------------------------------------------------------------------

pub struct ReportsWindow {
    pub kind: ReportKind,
    gran: Granularity,
    days: String,
    status: Option<String>,
    pub custom: CustomSpec,
}

impl Default for ReportsWindow {
    fn default() -> Self {
        let custom = LAST_SPEC.with(|s| s.borrow().clone()).unwrap_or_default();
        Self { kind: ReportKind::ProjectOverview, gran: Granularity::Week, days: "14".into(), status: None, custom }
    }
}

impl ReportsWindow {
    pub fn show(&mut self, ctx: &Context, doc: &Document) -> bool {
        let mut open = true;
        let p = &doc.project;
        egui::Window::new(crate::tr("Reports"))
            .id(egui::Id::new("reports_window"))
            .pivot(egui::Align2::CENTER_TOP)
            .default_pos(ctx.content_rect().center_top() + egui::vec2(0.0, 90.0))
            .default_size([900.0, 560.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(crate::tr("Report:"));
                    egui::ComboBox::from_id_salt("rep_kind").width(260.0).selected_text(self.kind.label()).show_ui(ui, |ui| {
                        for k in ReportKind::ALL {
                            ui.selectable_value(&mut self.kind, k, k.label());
                        }
                    });
                    if self.kind == ReportKind::CashFlow {
                        ui.label(crate::tr("Period:"));
                        ui.selectable_value(&mut self.gran, Granularity::Week, crate::tr("Week"));
                        ui.selectable_value(&mut self.gran, Granularity::Month, crate::tr("Month"));
                    }
                    if self.kind == ReportKind::UpcomingTasks {
                        ui.label(crate::tr("Starting within (days):"));
                        ui.add(egui::TextEdit::singleline(&mut self.days).desired_width(40.0));
                    }
                });
                if self.kind == ReportKind::Custom {
                    custom_editor(ui, &mut self.custom);
                    let spec = self.custom.clone();
                    LAST_SPEC.with(|s| *s.borrow_mut() = Some(spec));
                }
                let days: i64 = self.days.trim().parse().unwrap_or(14);
                let table =
                    if self.kind == ReportKind::Custom { build_custom(p, &self.custom) } else { build(p, self.kind, self.gran, days) };
                ui.horizontal(|ui| {
                    if ui.button(crate::tr("Export HTML…")).on_hover_text(crate::tr("Publish the report as a web page")).clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("HTML", &["html", "htm"])
                            .set_file_name(format!("{} - {}.html", p.name, table.title))
                            .save_file()
                    {
                        self.status = Some(match std::fs::write(&path, to_html(p, &table)) {
                            Ok(()) => crate::tr_dyn(&format!("Exported {}", path.display())),
                            Err(e) => crate::tr_dyn(&format!("Could not save {}: {}", path.display(), e)),
                        });
                    }
                    if ui
                        .button(crate::tr("Export to Excel (CSV)…"))
                        .on_hover_text(crate::tr("Save the report as a CSV table for Excel"))
                        .clicked()
                        && let Some(path) = rfd::FileDialog::new()
                            .add_filter("CSV", &["csv"])
                            .set_file_name(format!("{} - {}.csv", p.name, table.title))
                            .save_file()
                    {
                        self.status = Some(match std::fs::write(&path, to_csv(&table)) {
                            Ok(()) => crate::tr_dyn(&format!("Exported {}", path.display())),
                            Err(e) => crate::tr_dyn(&format!("Could not save {}: {}", path.display(), e)),
                        });
                    }
                    if let Some(s) = &self.status {
                        ui.label(RichText::new(s).color(theme::pal().TEXT_WEAK));
                    }
                });
                ui.separator();
                ui.heading(&table.title);
                if let Some(n) = &table.note {
                    ui.label(RichText::new(n).color(theme::pal().TEXT_WEAK));
                }
                if !table.cash.is_empty() {
                    cash_chart(ui, &table.cash, p);
                }
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    if table.rows.is_empty() {
                        ui.label(RichText::new(crate::tr("No tasks match this report.")).color(theme::pal().TEXT_WEAK));
                        return;
                    }
                    // Width of right-aligned columns: the longest text in them (≈7 px per character).
                    let widths: Vec<f32> = (0..table.headers.len())
                        .map(|c| {
                            let longest = std::iter::once(table.headers.get(c))
                                .chain(table.rows.iter().map(|r| r.get(c)))
                                .flatten()
                                .map(|t| t.chars().count())
                                .max()
                                .unwrap_or(4);
                            (longest as f32 * 7.2 + 6.0).max(40.0)
                        })
                        .collect();
                    egui::Grid::new("rep_grid").striped(true).num_columns(table.headers.len()).spacing([18.0, 4.0]).show(ui, |ui| {
                        for h in &table.headers {
                            ui.label(RichText::new(h).strong().color(theme::pal().ACCENT_MID));
                        }
                        ui.end_row();
                        for (row, bold) in table.rows.iter().zip(&table.bold) {
                            for (k, (c, right)) in row.iter().zip(&table.right).enumerate() {
                                let text = if *bold { RichText::new(c).strong() } else { RichText::new(c) };
                                if *right {
                                    ui.allocate_ui_with_layout(
                                        egui::vec2(widths.get(k).copied().unwrap_or(110.0), 18.0),
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| ui.label(text),
                                    );
                                } else {
                                    ui.label(text);
                                }
                            }
                            ui.end_row();
                        }
                    });
                });
            });
        open
    }
}

fn cash_chart(ui: &mut egui::Ui, rows: &[CashFlowRow], p: &Project) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(860.0), 170.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let pal = theme::pal();
    painter.rect_filled(rect, 4.0, pal.PANEL_BG);
    let inner = rect.shrink2(egui::vec2(12.0, 16.0));
    let max_cum = rows.last().map(|r| r.cumulative).unwrap_or(0.0).max(1.0) as f32;
    let max_bar = rows.iter().map(|r| r.cost).fold(0.0, f64::max).max(1.0) as f32;
    let n = rows.len().max(1) as f32;
    let bw = inner.width() / n;
    let mut pts = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let x = inner.left() + i as f32 * bw;
        let h = inner.height() * r.cost as f32 / max_bar;
        painter.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x + bw * 0.15, inner.bottom() - h), egui::pos2(x + bw * 0.85, inner.bottom())),
            1.0,
            pal.ACCENT_MID,
        );
        pts.push(egui::pos2(x + bw / 2.0, inner.bottom() - inner.height() * r.cumulative as f32 / max_cum));
    }
    painter.add(egui::Shape::line(pts, egui::Stroke::new(2.5, pal.DEADLINE)));
    painter.text(
        rect.right_top() + egui::vec2(-8.0, 4.0),
        egui::Align2::RIGHT_TOP,
        format!("{}: {}", crate::tr("Cumulative Cost"), format::money(p, rows.last().map(|r| r.cumulative).unwrap_or(0.0))),
        egui::FontId::proportional(12.0),
        pal.TEXT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_report_builds_and_exports() {
        let mut p = plancraft_core::sample::demo_project(chrono::NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
        p.set_baseline(&[]);
        for k in ReportKind::ALL {
            let t = build(&p, k, Granularity::Week, 14);
            assert!(t.rows.iter().all(|r| r.len() == t.headers.len()), "{k:?}");
            assert!(to_html(&p, &t).contains("<table>"));
            assert!(to_csv(&t).lines().count() >= 1);
        }
    }

    #[test]
    fn custom_report_filters_sorts_and_lists_assignments() {
        let p = plancraft_core::sample::demo_project(chrono::NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
        let spec = CustomSpec {
            filter: TaskFilter::InProgress,
            sort: CustomSort::PercentComplete,
            assignments: true,
            summaries: false,
            ..Default::default()
        };
        let t = build_custom(&p, &spec);
        assert!(t.rows.iter().all(|r| r.len() == t.headers.len()));
        let in_progress =
            p.tasks.iter().enumerate().filter(|(i, x)| !p.has_children(*i) && x.percent_complete > 0 && x.percent_complete < 100).count();
        let task_rows = t.rows.iter().filter(|r| !r.iter().any(|c| c.contains('↳'))).count();
        assert_eq!(task_rows, in_progress + 1, "{:?}", t.rows);
        assert!(t.rows.iter().any(|r| r.iter().any(|c| c.contains('↳'))));
        assert!(t.headers.len() >= 4);
        for s in CustomSort::ALL {
            for tb in TaskTable::ALL {
                let t = build_custom(&p, &CustomSpec { sort: s, table: tb, assignments: true, ..Default::default() });
                assert!(t.rows.iter().all(|r| r.len() == t.headers.len()));
            }
        }
    }
}
