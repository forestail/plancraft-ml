//! Built-in project templates ("File > New from Template"). User templates are ordinary
//! `.plancraft` files kept in the templates folder.

use chrono::NaiveDate;

use crate::i18n::tr;
use crate::model::*;
use crate::schedule::recalculate;

/// Built-in templates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Template {
    SoftwareDevelopment,
    OfficeMove,
    EventPlanning,
}

impl Template {
    pub const ALL: [Template; 3] = [Template::SoftwareDevelopment, Template::OfficeMove, Template::EventPlanning];

    pub fn label(self) -> &'static str {
        match self {
            Template::SoftwareDevelopment => tr("Software Development"),
            Template::OfficeMove => tr("Office Move"),
            Template::EventPlanning => tr("Event Planning"),
        }
    }
}

/// (outline level, name, duration in days (0 = milestone), predecessor rows)
type Row = (u16, &'static str, f64, &'static str);

fn rows(t: Template) -> &'static [Row] {
    match t {
        Template::SoftwareDevelopment => &[
            (1, "Project preparation", 0.0, ""),
            (2, "Requirements analysis", 5.0, ""),
            (2, "Technical specification", 4.0, "2"),
            (2, "Specification approved", 0.0, "3"),
            (1, "Development", 0.0, ""),
            (2, "Prototyping", 6.0, "4"),
            (2, "Architecture design", 3.0, "4"),
            (2, "Coding", 15.0, "6,7"),
            (2, "Documentation", 5.0, "8SS+5d"),
            (1, "Testing", 0.0, ""),
            (2, "Unit testing", 4.0, "8FF+2d"),
            (2, "System testing", 5.0, "11"),
            (2, "Bug fixing", 4.0, "12SS+2d"),
            (1, "Deployment", 0.0, ""),
            (2, "Installation and training", 3.0, "13"),
            (2, "Pilot operation", 5.0, "15"),
            (2, "Project closed", 0.0, "16"),
        ],
        Template::OfficeMove => &[
            (1, "Planning the move", 0.0, ""),
            (2, "Choose new office", 10.0, ""),
            (2, "Sign the lease", 2.0, "2"),
            (2, "Plan the layout", 4.0, "3"),
            (1, "Preparation", 0.0, ""),
            (2, "Order furniture", 3.0, "4"),
            (2, "Network and phones", 5.0, "4"),
            (2, "Pack the equipment", 3.0, "6,7"),
            (1, "Move", 0.0, ""),
            (2, "Transport", 2.0, "8"),
            (2, "Set up workplaces", 3.0, "10"),
            (2, "New office open", 0.0, "11"),
        ],
        Template::EventPlanning => &[
            (1, "Concept", 0.0, ""),
            (2, "Goals and budget", 3.0, ""),
            (2, "Choose the venue", 5.0, "2"),
            (2, "Program", 4.0, "2"),
            (1, "Organisation", 0.0, ""),
            (2, "Invite speakers", 7.0, "4"),
            (2, "Registration and marketing", 10.0, "3,4"),
            (2, "Catering and equipment", 5.0, "3"),
            (1, "Event", 0.0, ""),
            (2, "Rehearsal", 1.0, "6,7,8"),
            (2, "Event day", 1.0, "10"),
            (2, "Report and feedback", 3.0, "11"),
            (2, "Event closed", 0.0, "12"),
        ],
    }
}

/// Builds a new project from a built-in template, starting on `start`.
pub fn build(t: Template, start: NaiveDate) -> Project {
    let mut p = Project::new(t.label(), start);
    let mpd = p.settings.time.minutes_per_day;
    let mut preds: Vec<(TaskIdRow, &'static str)> = Vec::new();
    for (level, name, days, pr) in rows(t) {
        let id = p.ids.next_task();
        let mut task = Task::new(id, tr(name), mpd);
        task.outline_level = *level;
        task.estimated = false;
        task.duration = (days * mpd as f64).round() as i64;
        if *days == 0.0 && *level > 1 {
            task.kind = TaskKind::Milestone;
        }
        p.insert_task(p.tasks.len(), task);
        if !pr.is_empty() {
            preds.push((TaskIdRow(id), pr));
        }
    }
    recalculate(&mut p);
    for (TaskIdRow(id), text) in preds {
        // Predecessor text uses English link codes; parse with the English reader.
        let text = text.replace('d', &format!(" {}", if crate::i18n::is_ru() { "дн" } else { "d" }));
        if let Ok(deps) = p.parse_predecessors(&text)
            && let Some(t) = p.task_mut(id)
        {
            t.predecessors = deps;
        }
    }
    recalculate(&mut p);
    p
}

struct TaskIdRow(crate::ids::TaskId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_schedule() {
        for t in Template::ALL {
            let p = build(t, NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
            assert!(p.tasks.len() > 10);
            assert!(p.tasks.iter().filter(|t| !t.sched.is_summary).skip(1).any(|t| !t.predecessors.is_empty()));
            assert!(p.sched.finish > p.sched.start);
        }
    }
}
