//! Exchange with other tools: Microsoft Project XML (MSPDI, written by Project's
//! "Save As > XML"), and task lists in CSV or Excel (.xlsx) tables shaped like Project's
//! "Microsoft Project Task List Import Template" (Task_Table / Resource_Table / Assignment_Table).
//!
//! The XML format follows Microsoft's published schema documentation for the Project XML
//! Data Interchange; no Microsoft code is used.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use chrono::{NaiveDate, NaiveDateTime, Weekday};

use crate::calendar::{CalendarException, TimeRange, WEEKDAYS};
use crate::duration::{Lag, Minutes};
use crate::ids::{ResourceId, TaskId};
use crate::limits::{self, MAX_FILE_BYTES};
use crate::model::*;
use crate::schedule::recalculate;

// ---------------------------------------------------------------------------
// A tiny XML tree
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct Node {
    name: String,
    text: String,
    children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
    fn get(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.trim()).filter(|s| !s.is_empty())
    }
    fn int(&self, name: &str) -> Option<i64> {
        self.get(name).and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite()).map(|v| v as i64)
    }
    fn num(&self, name: &str) -> Option<f64> {
        self.get(name).and_then(|s| s.parse::<f64>().ok()).filter(|v| v.is_finite())
    }
    fn flag(&self, name: &str) -> bool {
        matches!(self.get(name), Some("1") | Some("true"))
    }
    fn date(&self, name: &str) -> Option<NaiveDateTime> {
        self.get(name).and_then(parse_xml_datetime)
    }
}

fn parse_xml(text: &str) -> Result<Node, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Node> = vec![Node::default()];
    let mut count = 0usize;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                count += 1;
                if count > 5_000_000 || stack.len() > 64 {
                    return Err("the XML file is too large or too deeply nested".into());
                }
                let name = e.local_name().as_ref().to_string();
                stack.push(Node { name, ..Node::default() });
            }
            Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                if let Some(top) = stack.last_mut() {
                    top.children.push(Node { name, ..Node::default() });
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.into_inner());
                }
            }
            Ok(Event::CData(t)) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t.into_inner());
                }
            }
            Ok(Event::GeneralRef(r)) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(c)) => c.to_string(),
                    _ => match r.into_inner().as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        _ => String::new(),
                    },
                };
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&resolved);
                }
            }
            Ok(Event::End(_)) => {
                if stack.len() > 1
                    && let Some(n) = stack.pop()
                    && let Some(parent) = stack.last_mut()
                {
                    parent.children.push(n);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("XML error at position {}: {e}", reader.buffer_position())),
        }
    }
    let root = stack.into_iter().next().unwrap_or_default();
    root.children.into_iter().next().ok_or_else(|| "the file is empty".to_string())
}

fn parse_xml_datetime(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    for f in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(d) = NaiveDateTime::parse_from_str(s, f) {
            return Some(limits::clamp_datetime(d));
        }
    }
    NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(8, 0, 0)).map(limits::clamp_datetime)
}

/// Parses an XML duration such as `PT40H0M0S` (or `P2DT4H`) into minutes.
fn parse_xml_duration(s: &str) -> Option<Minutes> {
    let s = s.trim();
    let neg = s.starts_with('-');
    let s = s.trim_start_matches('-').strip_prefix('P')?;
    let (date_part, time_part) = s.split_once('T').unwrap_or((s, ""));
    let mut total = 0.0f64;
    let mut num = String::new();
    for c in date_part.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let v: f64 = num.parse().ok()?;
            num.clear();
            total += match c {
                'Y' => v * 365.0 * 1440.0,
                'M' => v * 30.0 * 1440.0,
                'W' => v * 7.0 * 1440.0,
                'D' => v * 24.0 * 60.0,
                _ => 0.0,
            };
        }
    }
    for c in time_part.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
        } else {
            let v: f64 = num.parse().ok()?;
            num.clear();
            total += match c {
                'H' => v * 60.0,
                'M' => v,
                'S' => v / 60.0,
                _ => 0.0,
            };
        }
    }
    let m = limits::clamp_minutes(total.round() as Minutes);
    Some(if neg { -m } else { m })
}

fn xml_duration(m: Minutes) -> String {
    let m = m.max(0);
    format!("PT{}H{}M0S", m / 60, m % 60)
}

fn xml_dt(d: NaiveDateTime) -> String {
    d.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// MSPDI import
// ---------------------------------------------------------------------------

fn constraint_from_code(c: i64) -> ConstraintKind {
    match c {
        1 => ConstraintKind::Alap,
        2 => ConstraintKind::Mso,
        3 => ConstraintKind::Mfo,
        4 => ConstraintKind::Snet,
        5 => ConstraintKind::Snlt,
        6 => ConstraintKind::Fnet,
        7 => ConstraintKind::Fnlt,
        _ => ConstraintKind::Asap,
    }
}

fn constraint_code(k: ConstraintKind) -> i64 {
    match k {
        ConstraintKind::Asap => 0,
        ConstraintKind::Alap => 1,
        ConstraintKind::Mso => 2,
        ConstraintKind::Mfo => 3,
        ConstraintKind::Snet => 4,
        ConstraintKind::Snlt => 5,
        ConstraintKind::Fnet => 6,
        ConstraintKind::Fnlt => 7,
    }
}

fn accrual_from_code(c: Option<i64>) -> Accrual {
    match c {
        Some(1) => Accrual::Start,
        Some(2) => Accrual::End,
        _ => Accrual::Prorated,
    }
}

fn accrual_code(a: Accrual) -> i64 {
    match a {
        Accrual::Start => 1,
        Accrual::End => 2,
        Accrual::Prorated => 3,
    }
}

fn link_from_code(c: i64) -> LinkType {
    match c {
        0 => LinkType::FF,
        2 => LinkType::SF,
        3 => LinkType::SS,
        _ => LinkType::FS,
    }
}

fn link_code(t: LinkType) -> i64 {
    match t {
        LinkType::FF => 0,
        LinkType::FS => 1,
        LinkType::SF => 2,
        LinkType::SS => 3,
    }
}

fn weekday_from_type(t: i64) -> Option<Weekday> {
    // MSPDI DayType: 1 = Sunday … 7 = Saturday (0 = exception).
    match t {
        1 => Some(Weekday::Sun),
        2 => Some(Weekday::Mon),
        3 => Some(Weekday::Tue),
        4 => Some(Weekday::Wed),
        5 => Some(Weekday::Thu),
        6 => Some(Weekday::Fri),
        7 => Some(Weekday::Sat),
        _ => None,
    }
}

fn day_type(wd: Weekday) -> i64 {
    match wd {
        Weekday::Sun => 1,
        Weekday::Mon => 2,
        Weekday::Tue => 3,
        Weekday::Wed => 4,
        Weekday::Thu => 5,
        Weekday::Fri => 6,
        Weekday::Sat => 7,
    }
}

fn hhmm(s: &str) -> Option<u32> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.parse().ok()?;
    let m: u32 = it.next().unwrap_or("0").parse().ok()?;
    (h <= 24 && m < 60).then_some(h * 60 + m)
}

fn working_times(n: &Node) -> Vec<TimeRange> {
    let mut v = Vec::new();
    if let Some(wt) = n.child("WorkingTimes") {
        for w in wt.all("WorkingTime") {
            if let (Some(a), Some(b)) = (w.get("FromTime").and_then(hhmm), w.get("ToTime").and_then(hhmm)) {
                // "00:00" as an end time means midnight.
                let b = if b == 0 { 1440 } else { b };
                if a < b {
                    v.push(TimeRange::new(a, b));
                }
            }
        }
    }
    v
}

/// Reads a Microsoft Project XML document.
pub fn import_mspdi(text: &str) -> Result<Project, String> {
    let root = parse_xml(text)?;
    if root.name != "Project" {
        return Err("this is not a Microsoft Project XML file".into());
    }
    let start = root.date("StartDate").unwrap_or_else(|| chrono::Local::now().naive_local());
    let name = root.get("Title").or(root.get("Name")).unwrap_or("Project").trim_end_matches(".xml").to_string();
    let mut p = Project::new(name, start.date());
    p.start = start;
    p.author = root.get("Author").unwrap_or_default().to_string();
    p.company = root.get("Company").unwrap_or_default().to_string();
    p.manager = root.get("Manager").unwrap_or_default().to_string();
    if let Some(v) = root.int("MinutesPerDay").filter(|v| *v > 0) {
        p.settings.time.minutes_per_day = v;
    }
    if let Some(v) = root.int("MinutesPerWeek").filter(|v| *v > 0) {
        p.settings.time.minutes_per_week = v;
    }
    if let Some(v) = root.int("DaysPerMonth").filter(|v| *v > 0) {
        p.settings.time.days_per_month = v;
    }
    if let Some(c) = root.get("CurrencySymbol") {
        p.settings.currency = c.to_string();
    }
    if root.get("ScheduleFromStart") == Some("0") {
        p.settings.schedule_from_finish = true;
        p.settings.finish_date = root.date("FinishDate");
    }
    if let Some(d) = root.get("StatusDate").and_then(parse_xml_datetime) {
        p.status_date = Some(d.date());
    }

    // Project calendar: regular week and exceptions.
    let cal_uid = root.get("CalendarUID").map(str::to_string);
    if let Some(cals) = root.child("Calendars") {
        let base = cals
            .all("Calendar")
            .find(|c| cal_uid.as_deref().is_some_and(|u| c.get("UID") == Some(u)))
            .or_else(|| cals.all("Calendar").find(|c| c.flag("IsBaseCalendar")));
        if let Some(c) = base
            && let Some(pc) = p.calendars.iter_mut().find(|x| x.id == p.calendar)
        {
            if let Some(wds) = c.child("WeekDays") {
                for wd in wds.all("WeekDay") {
                    let t = wd.int("DayType").unwrap_or(-1);
                    if let Some(day) = weekday_from_type(t) {
                        *pc.week.day_mut(day) = if wd.flag("DayWorking") { working_times(wd) } else { Vec::new() };
                    } else if t == 0
                        && let Some(tp) = wd.child("TimePeriod")
                        && let (Some(a), Some(b)) = (tp.date("FromDate"), tp.date("ToDate"))
                    {
                        let mut e = CalendarException::non_working(wd.get("Name").unwrap_or("Exception"), a.date(), b.date());
                        if wd.flag("DayWorking") {
                            e.working = working_times(wd);
                        }
                        pc.exceptions.push(e);
                    }
                }
            }
            if let Some(exs) = c.child("Exceptions") {
                for ex in exs.all("Exception") {
                    if let Some(tp) = ex.child("TimePeriod")
                        && let (Some(a), Some(b)) = (tp.date("FromDate"), tp.date("ToDate"))
                    {
                        let mut e = CalendarException::non_working(ex.get("Name").unwrap_or("Exception"), a.date(), b.date());
                        if ex.flag("DayWorking") {
                            e.working = working_times(ex);
                        }
                        pc.exceptions.push(e);
                    }
                }
            }
        }
    }

    // Tasks in ID order; UID 0 / outline level 0 is the project summary task.
    let mut task_nodes: Vec<&Node> = root.child("Tasks").map(|t| t.all("Task").collect()).unwrap_or_default();
    task_nodes.sort_by_key(|t| t.int("ID").unwrap_or(0));
    let mut uid_map: HashMap<i64, TaskId> = HashMap::new();
    for tn in &task_nodes {
        let level = tn.int("OutlineLevel").unwrap_or(1);
        if level < 1 || tn.flag("IsNull") || tn.int("UID") == Some(0) {
            continue;
        }
        if p.tasks.len() >= limits::MAX_TASKS {
            break;
        }
        let id = p.ids.next_task();
        let mut t = Task::new(id, tn.get("Name").unwrap_or_default(), p.settings.time.minutes_per_day);
        t.outline_level = u16::try_from(level.clamp(1, i64::from(limits::MAX_OUTLINE_LEVEL))).unwrap_or(1);
        t.duration = tn.get("Duration").and_then(parse_xml_duration).unwrap_or(0).max(0);
        t.estimated = tn.flag("Estimated");
        if tn.flag("Milestone") {
            t.kind = TaskKind::Milestone;
        }
        t.task_type = match tn.int("Type") {
            Some(1) => TaskType::FixedDuration,
            Some(2) => TaskType::FixedWork,
            _ => TaskType::FixedUnits,
        };
        t.effort_driven = tn.flag("EffortDriven");
        t.priority = u16::try_from(tn.int("Priority").unwrap_or(500).clamp(0, 1000)).unwrap_or(500);
        t.percent_complete = u8::try_from(tn.int("PercentComplete").unwrap_or(0).clamp(0, 100)).unwrap_or(0);
        t.actual_start = tn.date("ActualStart");
        t.actual_finish = tn.date("ActualFinish");
        let ck = constraint_from_code(tn.int("ConstraintType").unwrap_or(0));
        t.constraint = Constraint { kind: ck, date: if ck.needs_date() { tn.date("ConstraintDate") } else { None } };
        t.deadline = tn.date("Deadline");
        t.fixed_cost = tn.num("FixedCost").unwrap_or(0.0).clamp(-limits::MAX_MONEY, limits::MAX_MONEY);
        t.fixed_cost_accrual = accrual_from_code(tn.int("FixedCostAccrual"));
        t.notes = tn.get("Notes").unwrap_or_default().to_string();
        t.active = tn.get("Active") != Some("0");
        if let Some(addr) = tn.get("HyperlinkAddress") {
            t.hyperlink = Some(Hyperlink { text: tn.get("Hyperlink").unwrap_or(addr).to_string(), address: addr.to_string() });
        }
        if tn.flag("Manual") {
            t.mode = ScheduleMode::Manual;
            t.manual_start = tn.date("ManualStart").or(tn.date("Start"));
            t.manual_finish = tn.date("ManualFinish").or(tn.date("Finish"));
        }
        for b in tn.all("Baseline") {
            let (Some(s), Some(f)) = (b.date("Start"), b.date("Finish")) else { continue };
            let bl = Baseline {
                start: s,
                finish: f,
                duration: b.get("Duration").and_then(parse_xml_duration).unwrap_or(0),
                work: b.get("Work").and_then(parse_xml_duration).unwrap_or(0),
                cost: b.num("Cost").unwrap_or(0.0) / 100.0,
                saved: s,
            };
            match b.int("Number").unwrap_or(0) {
                0 => t.baseline = Some(bl),
                n @ 1..=10 => {
                    t.baselines.insert(n as u8, bl);
                }
                _ => {}
            }
        }
        if let Some(uid) = tn.int("UID") {
            uid_map.insert(uid, id);
        }
        p.insert_task(p.tasks.len(), t);
    }
    // Links, now that every task has an id.
    for tn in &task_nodes {
        let Some(&id) = tn.int("UID").and_then(|u| uid_map.get(&u)) else { continue };
        let mut deps = Vec::new();
        for l in tn.all("PredecessorLink") {
            let Some(&pred) = l.int("PredecessorUID").and_then(|u| uid_map.get(&u)) else { continue };
            let kind = link_from_code(l.int("Type").unwrap_or(1));
            // LinkLag is in tenths of a minute; formats 19/20 are percentages (LinkLag × 0.1 %).
            let raw = l.int("LinkLag").unwrap_or(0);
            let lag = match l.int("LagFormat") {
                Some(19) | Some(20) => Lag::Percent(i32::try_from(raw / 10).unwrap_or(0)),
                Some(4) | Some(6) | Some(8) | Some(10) | Some(12) => Lag::Elapsed(limits::clamp_minutes(raw / 10)),
                _ if raw == 0 => Lag::None,
                _ => Lag::Working(limits::clamp_minutes(raw / 10)),
            };
            if deps.len() < limits::MAX_LINKS_PER_TASK {
                deps.push(Dependency { predecessor: TaskRef::Local(pred), kind, lag });
            }
        }
        if let Some(t) = p.task_mut(id) {
            t.predecessors = deps;
        }
    }

    // Resources (UID 0 is a placeholder).
    let mut res_map: HashMap<i64, ResourceId> = HashMap::new();
    if let Some(rs) = root.child("Resources") {
        for rn in rs.all("Resource") {
            let Some(name) = rn.get("Name") else { continue };
            if rn.int("UID") == Some(0) || rn.flag("IsNull") || p.resources.len() >= limits::MAX_RESOURCES {
                continue;
            }
            let id = p.add_resource(name);
            if let Some(r) = p.resource_mut(id) {
                r.kind = match rn.int("Type") {
                    Some(0) => ResourceType::Material,
                    Some(2) => ResourceType::Cost,
                    _ => ResourceType::Work,
                };
                if let Some(i) = rn.get("Initials") {
                    r.initials = i.to_string();
                }
                r.group = rn.get("Group").unwrap_or_default().to_string();
                r.email = rn.get("EmailAddress").unwrap_or_default().to_string();
                r.material_label = rn.get("MaterialLabel").unwrap_or_default().to_string();
                r.max_units = rn.num("MaxUnits").unwrap_or(1.0).clamp(0.0, limits::MAX_UNITS);
                r.standard_rate = rn.num("StandardRate").unwrap_or(0.0).clamp(0.0, limits::MAX_MONEY);
                r.overtime_rate = rn.num("OvertimeRate").unwrap_or(0.0).clamp(0.0, limits::MAX_MONEY);
                r.cost_per_use = rn.num("CostPerUse").unwrap_or(0.0).clamp(0.0, limits::MAX_MONEY);
                r.accrue_at = accrual_from_code(rn.int("AccrueAt").map(|c| if c == 3 { 3 } else { c }));
                r.notes = rn.get("Notes").unwrap_or_default().to_string();
            }
            if let Some(uid) = rn.int("UID") {
                res_map.insert(uid, id);
            }
        }
    }
    // Assignments.
    if let Some(asg) = root.child("Assignments") {
        for an in asg.all("Assignment") {
            let (Some(&task), Some(&res)) =
                (an.int("TaskUID").and_then(|u| uid_map.get(&u)), an.int("ResourceUID").and_then(|u| res_map.get(&u)))
            else {
                continue;
            };
            if p.assignments.len() >= limits::MAX_ASSIGNMENTS {
                break;
            }
            let units = an.num("Units").unwrap_or(1.0).clamp(0.0, limits::MAX_UNITS);
            let work = an.get("Work").and_then(parse_xml_duration).unwrap_or(0).max(0);
            let id = p.ids.next_assignment();
            let cost = an.num("Cost").unwrap_or(0.0) / 100.0;
            let kind = p.resource(res).map(|r| r.kind);
            p.assignments.push(Assignment {
                id,
                task,
                resource: res,
                units,
                work: if kind == Some(ResourceType::Work) { work } else { 0 },
                actual_work: an.get("ActualWork").and_then(parse_xml_duration).unwrap_or(0).max(0),
                cost: if kind == Some(ResourceType::Cost) { cost } else { 0.0 },
                contour: WorkContour::Flat,
            });
        }
    }
    limits::sanitize(&mut p);
    recalculate(&mut p);
    Ok(p)
}

// ---------------------------------------------------------------------------
// MSPDI export
// ---------------------------------------------------------------------------

/// Writes a Microsoft Project XML document (open it in Project with File > Open, type XML).
pub fn export_mspdi(p: &Project) -> String {
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Project xmlns=\"http://schemas.microsoft.com/project\">\n",
    );
    let tag = |s: &mut String, indent: &str, name: &str, v: &str| {
        s.push_str(&format!("{indent}<{name}>{}</{name}>\n", esc(v)));
    };
    tag(&mut s, "  ", "SaveVersion", "14");
    tag(&mut s, "  ", "Name", &format!("{}.xml", p.name));
    tag(&mut s, "  ", "Title", &p.name);
    tag(&mut s, "  ", "Company", &p.company);
    tag(&mut s, "  ", "Author", &p.author);
    tag(&mut s, "  ", "Manager", &p.manager);
    tag(&mut s, "  ", "ScheduleFromStart", if p.settings.schedule_from_finish { "0" } else { "1" });
    tag(&mut s, "  ", "StartDate", &xml_dt(p.sched.start.unwrap_or(p.start)));
    tag(&mut s, "  ", "FinishDate", &xml_dt(p.sched.finish.unwrap_or(p.start)));
    if let Some(d) = p.status_date {
        tag(&mut s, "  ", "StatusDate", &format!("{}T17:00:00", d.format("%Y-%m-%d")));
    }
    tag(&mut s, "  ", "CalendarUID", "1");
    tag(&mut s, "  ", "MinutesPerDay", &p.settings.time.minutes_per_day.to_string());
    tag(&mut s, "  ", "MinutesPerWeek", &p.settings.time.minutes_per_week.to_string());
    tag(&mut s, "  ", "DaysPerMonth", &p.settings.time.days_per_month.to_string());
    tag(&mut s, "  ", "CurrencySymbol", &p.settings.currency);

    // Calendar 1: the project calendar.
    let base = p.base_calendar();
    s.push_str("  <Calendars>\n    <Calendar>\n      <UID>1</UID>\n");
    tag(&mut s, "      ", "Name", &base.name);
    s.push_str("      <IsBaseCalendar>1</IsBaseCalendar>\n      <WeekDays>\n");
    for wd in WEEKDAYS {
        let ranges = base.week.day(wd);
        s.push_str(&format!(
            "        <WeekDay>\n          <DayType>{}</DayType>\n          <DayWorking>{}</DayWorking>\n",
            day_type(wd),
            u8::from(!ranges.is_empty())
        ));
        if !ranges.is_empty() {
            s.push_str("          <WorkingTimes>\n");
            for r in ranges {
                s.push_str(&format!(
                    "            <WorkingTime><FromTime>{:02}:{:02}:00</FromTime><ToTime>{:02}:{:02}:00</ToTime></WorkingTime>\n",
                    r.start / 60,
                    r.start % 60,
                    (r.end / 60) % 24,
                    r.end % 60
                ));
            }
            s.push_str("          </WorkingTimes>\n");
        }
        s.push_str("        </WeekDay>\n");
    }
    s.push_str("      </WeekDays>\n");
    if !base.exceptions.is_empty() {
        s.push_str("      <Exceptions>\n");
        for e in &base.exceptions {
            s.push_str("        <Exception>\n");
            tag(&mut s, "          ", "Name", &e.name);
            s.push_str(&format!(
                "          <TimePeriod><FromDate>{}T00:00:00</FromDate><ToDate>{}T23:59:00</ToDate></TimePeriod>\n          <DayWorking>{}</DayWorking>\n",
                e.start.format("%Y-%m-%d"),
                e.end.format("%Y-%m-%d"),
                u8::from(!e.working.is_empty())
            ));
            if !e.working.is_empty() {
                s.push_str("          <WorkingTimes>\n");
                for r in &e.working {
                    s.push_str(&format!(
                        "            <WorkingTime><FromTime>{:02}:{:02}:00</FromTime><ToTime>{:02}:{:02}:00</ToTime></WorkingTime>\n",
                        r.start / 60,
                        r.start % 60,
                        (r.end / 60) % 24,
                        r.end % 60
                    ));
                }
                s.push_str("          </WorkingTimes>\n");
            }
            s.push_str("        </Exception>\n");
        }
        s.push_str("      </Exceptions>\n");
    }
    s.push_str("    </Calendar>\n  </Calendars>\n");

    // Tasks: UID = our task id (stable), ID = row number.
    s.push_str("  <Tasks>\n");
    for (i, t) in p.tasks.iter().enumerate() {
        let ind = "      ";
        s.push_str("    <Task>\n");
        tag(&mut s, ind, "UID", &t.id.0.to_string());
        tag(&mut s, ind, "ID", &(i + 1).to_string());
        tag(&mut s, ind, "Name", &t.name);
        tag(&mut s, ind, "Active", if t.active { "1" } else { "0" });
        tag(&mut s, ind, "Manual", if t.mode == ScheduleMode::Manual { "1" } else { "0" });
        tag(
            &mut s,
            ind,
            "Type",
            match t.task_type {
                TaskType::FixedUnits => "0",
                TaskType::FixedDuration => "1",
                TaskType::FixedWork => "2",
            },
        );
        tag(&mut s, ind, "WBS", &p.wbs(i));
        tag(&mut s, ind, "OutlineLevel", &t.outline_level.to_string());
        tag(&mut s, ind, "Priority", &t.priority.to_string());
        if let Some(d) = t.sched.start {
            tag(&mut s, ind, "Start", &xml_dt(d));
        }
        if let Some(d) = t.sched.finish {
            tag(&mut s, ind, "Finish", &xml_dt(d));
        }
        tag(&mut s, ind, "Duration", &xml_duration(t.sched.duration));
        tag(&mut s, ind, "DurationFormat", if t.estimated { "39" } else { "7" });
        tag(&mut s, ind, "Work", &xml_duration(t.sched.work));
        tag(&mut s, ind, "Estimated", if t.estimated { "1" } else { "0" });
        tag(&mut s, ind, "Milestone", if t.is_milestone() { "1" } else { "0" });
        tag(&mut s, ind, "Summary", if t.sched.is_summary { "1" } else { "0" });
        tag(&mut s, ind, "Critical", if t.sched.critical { "1" } else { "0" });
        tag(&mut s, ind, "EffortDriven", if t.effort_driven { "1" } else { "0" });
        tag(&mut s, ind, "PercentComplete", &t.percent_complete.to_string());
        tag(&mut s, ind, "Cost", &format!("{:.0}", t.sched.cost * 100.0));
        tag(&mut s, ind, "FixedCost", &format!("{:.2}", t.fixed_cost));
        tag(&mut s, ind, "FixedCostAccrual", &accrual_code(t.fixed_cost_accrual).to_string());
        if let Some(d) = t.actual_start {
            tag(&mut s, ind, "ActualStart", &xml_dt(d));
        }
        if let Some(d) = t.actual_finish {
            tag(&mut s, ind, "ActualFinish", &xml_dt(d));
        }
        tag(&mut s, ind, "ConstraintType", &constraint_code(t.constraint.kind).to_string());
        if let Some(d) = t.constraint.date {
            tag(&mut s, ind, "ConstraintDate", &xml_dt(d));
        }
        if let Some(d) = t.deadline {
            tag(&mut s, ind, "Deadline", &xml_dt(d));
        }
        if !t.notes.is_empty() {
            tag(&mut s, ind, "Notes", &t.notes);
        }
        if let Some(h) = &t.hyperlink {
            tag(&mut s, ind, "Hyperlink", &h.text);
            tag(&mut s, ind, "HyperlinkAddress", &h.address);
        }
        if t.mode == ScheduleMode::Manual {
            if let Some(d) = t.manual_start {
                tag(&mut s, ind, "ManualStart", &xml_dt(d));
            }
            if let Some(d) = t.manual_finish {
                tag(&mut s, ind, "ManualFinish", &xml_dt(d));
            }
        }
        // Milestone member tasks schedule like finish-to-start predecessors in Project.
        let mut links: Vec<Dependency> = t.predecessors.clone();
        for m in &t.members {
            if !links.iter().any(|d| d.predecessor == *m) {
                links.push(Dependency { predecessor: *m, kind: LinkType::FS, lag: Lag::None });
            }
        }
        for d in &links {
            let TaskRef::Local(pred) = d.predecessor else { continue };
            let (lag, fmt) = match d.lag {
                Lag::None => (0, 7),
                Lag::Working(m) => (m * 10, 7),
                Lag::Elapsed(m) => (m * 10, 8),
                Lag::Percent(pc) => (i64::from(pc) * 10, 19),
            };
            s.push_str(&format!(
                "      <PredecessorLink><PredecessorUID>{}</PredecessorUID><Type>{}</Type><CrossProject>0</CrossProject><LinkLag>{lag}</LinkLag><LagFormat>{fmt}</LagFormat></PredecessorLink>\n",
                pred.0,
                link_code(d.kind)
            ));
        }
        let mut baselines: Vec<(u8, &Baseline)> = t.baseline.iter().map(|b| (0u8, b)).collect();
        baselines.extend(t.baselines.iter().map(|(n, b)| (*n, b)));
        for (n, b) in baselines {
            s.push_str(&format!(
                "      <Baseline><Number>{n}</Number><Start>{}</Start><Finish>{}</Finish><Duration>{}</Duration><Work>{}</Work><Cost>{:.0}</Cost></Baseline>\n",
                xml_dt(b.start),
                xml_dt(b.finish),
                xml_duration(b.duration),
                xml_duration(b.work),
                b.cost * 100.0
            ));
        }
        s.push_str("    </Task>\n");
    }
    s.push_str("  </Tasks>\n  <Resources>\n");
    for (i, r) in p.resources.iter().enumerate() {
        let ind = "      ";
        s.push_str("    <Resource>\n");
        tag(&mut s, ind, "UID", &r.id.0.to_string());
        tag(&mut s, ind, "ID", &(i + 1).to_string());
        tag(&mut s, ind, "Name", &r.name);
        tag(
            &mut s,
            ind,
            "Type",
            match r.kind {
                ResourceType::Material => "0",
                ResourceType::Work => "1",
                ResourceType::Cost => "2",
            },
        );
        tag(&mut s, ind, "Initials", &r.initials);
        tag(&mut s, ind, "Group", &r.group);
        tag(&mut s, ind, "EmailAddress", &r.email);
        if !r.material_label.is_empty() {
            tag(&mut s, ind, "MaterialLabel", &r.material_label);
        }
        tag(&mut s, ind, "MaxUnits", &format!("{:.2}", r.max_units));
        tag(&mut s, ind, "StandardRate", &format!("{:.2}", r.standard_rate));
        tag(&mut s, ind, "StandardRateFormat", "2");
        tag(&mut s, ind, "OvertimeRate", &format!("{:.2}", r.overtime_rate));
        tag(&mut s, ind, "CostPerUse", &format!("{:.2}", r.cost_per_use));
        tag(&mut s, ind, "AccrueAt", &accrual_code(r.accrue_at).to_string());
        if !r.notes.is_empty() {
            tag(&mut s, ind, "Notes", &r.notes);
        }
        s.push_str("    </Resource>\n");
    }
    s.push_str("  </Resources>\n  <Assignments>\n");
    for a in &p.assignments {
        s.push_str(&format!(
            "    <Assignment><UID>{}</UID><TaskUID>{}</TaskUID><ResourceUID>{}</ResourceUID><Units>{:.2}</Units><Work>{}</Work><ActualWork>{}</ActualWork><Cost>{:.0}</Cost></Assignment>\n",
            a.id.0,
            a.task.0,
            a.resource.0,
            a.units,
            xml_duration(a.work),
            xml_duration(a.actual_work),
            a.cost * 100.0
        ));
    }
    s.push_str("  </Assignments>\n</Project>\n");
    s
}

// ---------------------------------------------------------------------------
// Tables (CSV / XLSX)
// ---------------------------------------------------------------------------

/// Splits CSV text into rows, detecting `;`, `,` or tab separators.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or("");
    let sep = [';', '\t', ','].into_iter().max_by_key(|c| first.matches(*c).count()).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
        } else if c == '"' && field.is_empty() {
            quoted = true;
        } else if c == sep {
            row.push(std::mem::take(&mut field));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut field));
            if row.iter().any(|f| !f.trim().is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
            if rows.len() > limits::MAX_TASKS * 2 {
                break;
            }
        } else {
            field.push(c);
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.trim().is_empty()) {
        rows.push(row);
    }
    rows
}

/// Column meaning, from English or Russian headers.
fn column_kind(h: &str) -> &'static str {
    let h = h.trim().to_lowercase();
    let h = h.as_str();
    match h {
        "id" | "ид" | "№" | "n" => "id",
        "name" | "task name" | "название" | "название задачи" | "задача" | "наименование" => "name",
        "duration" | "длительность" | "продолжительность" => "duration",
        "start" | "начало" | "start date" | "дата начала" => "start",
        "finish" | "окончание" | "finish date" | "дата окончания" => "finish",
        "predecessors" | "предшественники" => "predecessors",
        "outline level" | "уровень структуры" | "уровень" => "level",
        "notes" | "заметки" | "примечания" => "notes",
        "resource names" | "названия ресурсов" | "ресурсы" | "resources" => "resources",
        "milestone" | "веха" => "milestone",
        "% complete" | "% завершения" | "% выполнения" => "percent",
        "fixed cost" | "фиксированные затраты" => "fixed_cost",
        "deadline" | "крайний срок" => "deadline",
        _ => "",
    }
}

/// Builds a project from a task table (first row = headers); `resources` optionally lists
/// resources as (name, standard rate) rows.
pub fn import_task_table(name: &str, rows: &[Vec<String>], resources: &[Vec<String>]) -> Result<Project, String> {
    let header = rows.first().ok_or("the table is empty")?;
    let kinds: Vec<&str> = header.iter().map(|h| column_kind(h)).collect();
    if !kinds.contains(&"name") {
        return Err("the table needs a Name (Название) column".into());
    }
    let today = chrono::Local::now().date_naive();
    let mut p = Project::new(name, today);
    let col = |row: &[String], k: &str| -> Option<String> {
        kinds.iter().position(|x| *x == k).and_then(|i| row.get(i)).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    // Resource table: Name / Название, Standard Rate / Стандартная ставка, Type / Тип, Initials.
    if let Some(rh) = resources.first() {
        let rk: Vec<String> = rh.iter().map(|h| h.trim().to_lowercase()).collect();
        let find = |names: &[&str]| rk.iter().position(|h| names.contains(&h.as_str()));
        let (ni, rate, ty, ini, grp, maxu) = (
            find(&["name", "название", "название ресурса", "resource name"]),
            find(&["standard rate", "стандартная ставка", "ставка"]),
            find(&["type", "тип"]),
            find(&["initials", "краткое название"]),
            find(&["group", "группа"]),
            find(&["max units", "макс. единиц", "максимум единиц"]),
        );
        for r in resources.iter().skip(1) {
            let Some(n) = ni.and_then(|i| r.get(i)).map(|s| s.trim()).filter(|s| !s.is_empty()) else { continue };
            let id = p.add_resource(n);
            if let Some(res) = p.resource_mut(id) {
                if let Some(v) = rate.and_then(|i| r.get(i)).and_then(|s| {
                    crate::i18n::parse_f64(&s.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect::<String>())
                }) {
                    res.standard_rate = v.clamp(0.0, limits::MAX_MONEY);
                }
                if let Some(t) = ty.and_then(|i| r.get(i)).map(|s| s.trim().to_lowercase()) {
                    res.kind = if t.starts_with("mat") || t.starts_with("мат") {
                        ResourceType::Material
                    } else if t.starts_with("cost") || t.starts_with("затрат") {
                        ResourceType::Cost
                    } else {
                        ResourceType::Work
                    };
                }
                if let Some(v) = ini.and_then(|i| r.get(i)).filter(|s| !s.trim().is_empty()) {
                    res.initials = v.trim().to_string();
                }
                if let Some(v) = grp.and_then(|i| r.get(i)) {
                    res.group = v.trim().to_string();
                }
                if let Some(v) = maxu.and_then(|i| r.get(i)).and_then(|s| crate::i18n::parse_f64(s.trim().trim_end_matches('%'))) {
                    res.max_units = if v > 10.0 { v / 100.0 } else { v };
                }
            }
        }
    }
    let mut pending_preds: Vec<(TaskId, String)> = Vec::new();
    let mut pending_res: Vec<(TaskId, String)> = Vec::new();
    for row in rows.iter().skip(1) {
        let Some(n) = col(row, "name") else { continue };
        if p.tasks.len() >= limits::MAX_TASKS {
            break;
        }
        let id = p.ids.next_task();
        let mut t = Task::new(id, n, p.settings.time.minutes_per_day);
        if let Some(l) = col(row, "level").and_then(|s| s.parse::<u16>().ok()) {
            t.outline_level = l.clamp(1, limits::MAX_OUTLINE_LEVEL);
        }
        if let Some(d) = col(row, "duration").and_then(|s| p.parse_duration(&s)) {
            t.duration = d.0.max(0);
            t.estimated = d.1;
            if d.0 == 0 {
                t.kind = TaskKind::Milestone;
            }
        }
        if col(row, "milestone").is_some_and(|s| matches!(s.to_lowercase().as_str(), "yes" | "да" | "1" | "true")) {
            t.kind = TaskKind::Milestone;
            t.duration = 0;
        }
        if let Some(s) = col(row, "start").and_then(|s| p.parse_datetime(&s, false)) {
            t.constraint = Constraint { kind: ConstraintKind::Snet, date: Some(s) };
        }
        if let Some(s) = col(row, "deadline").and_then(|s| p.parse_datetime(&s, true)) {
            t.deadline = Some(s);
        }
        if let Some(v) = col(row, "percent").and_then(|s| crate::i18n::parse_f64(s.trim_end_matches('%'))) {
            t.percent_complete = v.clamp(0.0, 100.0) as u8;
        }
        if let Some(v) = col(row, "fixed_cost")
            .and_then(|s| crate::i18n::parse_f64(&s.chars().filter(|c| c.is_ascii_digit() || *c == '.' || *c == ',').collect::<String>()))
        {
            t.fixed_cost = v.clamp(0.0, limits::MAX_MONEY);
        }
        t.notes = col(row, "notes").unwrap_or_default();
        if let Some(pr) = col(row, "predecessors") {
            pending_preds.push((id, pr));
        }
        if let Some(r) = col(row, "resources") {
            pending_res.push((id, r));
        }
        p.insert_task(p.tasks.len(), t);
    }
    // Predecessors refer to row numbers, as in Project's import template.
    for (id, text) in pending_preds {
        if let Ok(deps) = p.parse_predecessors(&text)
            && let Some(t) = p.task_mut(id)
        {
            t.predecessors = deps;
        }
    }
    recalculate(&mut p);
    for (id, text) in pending_res {
        let _ = p.apply_resource_names(id, &text);
    }
    limits::sanitize(&mut p);
    recalculate(&mut p);
    Ok(p)
}

/// Reads all sheets of an .xlsx workbook as text rows.
/// A worksheet as (name, rows of cell text).
pub type Sheet = (String, Vec<Vec<String>>);

pub fn read_xlsx(path: &Path) -> Result<Vec<Sheet>, String> {
    use calamine::{Reader, Xlsx, open_workbook};
    let mut wb: Xlsx<_> = open_workbook(path).map_err(|e: calamine::XlsxError| e.to_string())?;
    let mut out = Vec::new();
    for name in wb.sheet_names().to_vec() {
        let Ok(range) = wb.worksheet_range(&name) else { continue };
        let rows: Vec<Vec<String>> = range
            .rows()
            .take(limits::MAX_TASKS * 2)
            .map(|r| {
                r.iter()
                    .map(|c| match c {
                        calamine::Data::DateTime(d) => {
                            // Excel serial date: days since 1899-12-30.
                            let v = d.as_f64();
                            NaiveDate::from_ymd_opt(1899, 12, 30)
                                .and_then(|base| base.and_hms_opt(0, 0, 0))
                                .and_then(|base| base.checked_add_signed(chrono::Duration::minutes((v * 1440.0).round() as i64)))
                                .map(|x| x.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| c.to_string())
                        }
                        calamine::Data::Float(f) if f.fract() == 0.0 => format!("{}", *f as i64),
                        _ => c.to_string(),
                    })
                    .collect()
            })
            .filter(|r: &Vec<String>| r.iter().any(|c| !c.trim().is_empty()))
            .collect();
        out.push((name, rows));
    }
    Ok(out)
}

fn read_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    if file.metadata().map_err(|e| e.to_string())?.len() > MAX_FILE_BYTES {
        return Err("the file is too large".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    // CSV from Russian Excel may be Windows-1251; fall back to a byte-wise conversion.
    Ok(match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => e.into_bytes().iter().map(|&b| cp1251(b)).collect(),
    })
}

fn cp1251(b: u8) -> char {
    match b {
        0x00..=0x7F => char::from(b),
        0xC0..=0xFF => char::from_u32(0x0410 + u32::from(b - 0xC0)).unwrap_or('?'),
        0xA8 => 'Ё',
        0xB8 => 'ё',
        0xB9 => '№',
        _ => '?',
    }
}

/// Imports a project from a `.xml` (MS Project), `.csv` or `.xlsx` file.
pub fn import_file(path: &Path) -> Result<Project, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "Project".into());
    match ext.as_str() {
        "xml" => {
            let mut p = import_mspdi(&read_text(path)?)?;
            if p.name.is_empty() || p.name == "Project" {
                p.name = stem;
            }
            Ok(p)
        }
        "csv" | "txt" => import_task_table(&stem, &parse_csv(&read_text(path)?), &[]),
        "xlsx" | "xlsm" => {
            let sheets = read_xlsx(path)?;
            let pick =
                |names: &[&str]| sheets.iter().find(|(n, _)| names.iter().any(|x| n.to_lowercase().contains(x))).map(|(_, r)| r.clone());
            let tasks = pick(&["task", "задач"]).or_else(|| sheets.first().map(|(_, r)| r.clone())).unwrap_or_default();
            let res = pick(&["resource", "ресурс"]).unwrap_or_default();
            import_task_table(&stem, &tasks, &res)
        }
        "mpp" => {
            Err("binary .mpp files cannot be read directly: in Microsoft Project use File > Save As > XML, then import the .xml file"
                .into())
        }
        _ => Err(format!("unsupported file type .{ext}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mspdi_round_trip_keeps_structure() {
        let mut p = crate::sample::demo_project(NaiveDate::from_ymd_opt(2026, 10, 5).unwrap());
        p.set_baseline(&[]);
        let xml = export_mspdi(&p);
        let q = import_mspdi(&xml).unwrap();
        assert_eq!(q.tasks.len(), p.tasks.len());
        assert_eq!(q.resources.len(), p.resources.len());
        assert_eq!(q.assignments.len(), p.assignments.len());
        for (a, b) in p.tasks.iter().zip(&q.tasks) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.outline_level, b.outline_level);
            assert_eq!(a.predecessors.len() + a.members.len(), b.predecessors.len(), "{}", a.name);
            assert_eq!(a.baseline.is_some(), b.baseline.is_some());
        }
        assert_eq!(q.sched.finish, p.sched.finish);
    }

    #[test]
    fn csv_import_with_russian_headers() {
        let csv = "\u{feff}Название;Длительность;Предшественники;Уровень структуры\nЭтап 1;;;1\nАнализ;3 дн;;2\nПроект;1,5 нед;2;2\nГотово;0;3;1\n";
        let p = import_task_table("T", &parse_csv(csv), &[]).unwrap();
        assert_eq!(p.tasks.len(), 4);
        assert!(p.tasks[0].sched.is_summary);
        assert_eq!(p.tasks[2].duration, 2400 * 3 / 2);
        assert!(p.tasks[3].is_milestone());
        assert_eq!(p.tasks[2].predecessors.len(), 1);
    }

    #[test]
    fn xml_durations() {
        assert_eq!(parse_xml_duration("PT40H0M0S"), Some(2400));
        assert_eq!(parse_xml_duration("PT8H30M0S"), Some(510));
        assert_eq!(parse_xml_duration("P1DT2H"), Some(1560));
        assert_eq!(xml_duration(510), "PT8H30M0S");
    }

    #[test]
    fn garbage_does_not_panic() {
        assert!(import_mspdi("").is_err());
        assert!(import_mspdi("<Project><Tasks><Task><UID>x</UID></Task></Tasks>").is_ok() || true);
        assert!(import_mspdi("<<<>>>").is_err() || true);
        let _ = parse_csv("\"unterminated");
    }
}
