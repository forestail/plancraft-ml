//! User commands issued from the menu bar, toolbar, menus, keyboard shortcuts and views.

use plancraft_core::model::ScheduleMode;

use crate::columns::TaskTable;
use crate::state::{TaskFilter, ViewKind};

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    // File
    NewProject,
    NewDemo,
    Open,
    OpenPath(std::path::PathBuf),
    Save,
    SaveAs,
    Close,
    Undo,
    Redo,
    ExportCsv,
    Import,
    ExportXml,
    PrintGantt,
    Templates,
    SaveTemplate,
    NewFromTemplate(plancraft_core::templates::Template),
    NewFromTemplateFile(std::path::PathBuf),
    Quit,
    StartPage,
    /// Restore a recovery copy (by its stem in the recovery folder).
    RestoreRecovered(String),
    /// Delete a recovery copy.
    DiscardRecovered(String),
    // Tasks
    InsertTask,
    InsertMilestone,
    InsertSummary,
    DeleteSelection,
    Indent,
    Outdent,
    MoveUp,
    MoveDown,
    LinkSelected,
    UnlinkSelected,
    SetPercent(u8),
    MarkOnTrack,
    ToggleActive,
    SetMode(ScheduleMode),
    TaskInfo,
    TaskNotes,
    ToggleTimeline,
    ScrollToTask,
    Cut,
    Copy,
    Paste,
    RecurringTask,
    Hyperlink,
    OpenHyperlink,
    // Resources
    InsertResource,
    DeleteResources,
    ResourceInfo,
    AssignResources,
    LevelAll,
    ClearLeveling,
    // Project
    ProjectInfo,
    ChangeWorkingTime,
    BaselineDialog(bool),
    UpdateProject,
    DefineWbs,
    Reports,
    CustomReport,
    SortResources(plancraft_core::extras::SortKey),
    UpdateProgressFromTime,
    ExternalLinks,
    RefreshExternal,
    Statistics,
    McpPanel,
    // View
    SetView(ViewKind),
    ZoomIn,
    ZoomOut,
    ZoomEntire,
    ZoomSelected,
    Filter(TaskFilter),
    Table(TaskTable),
    CollapseAll,
    ExpandAll,
    History,
    SetTheme(crate::theme::ThemeChoice),
    SetLang(plancraft_core::i18n::Lang),
    SetFont(String),
    About,
}

/// Context menu shared by the task table and the Gantt chart.
pub fn task_context_menu(ui: &mut egui::Ui, emit: &mut dyn FnMut(Command)) {
    let mut item = |ui: &mut egui::Ui, label: &str, cmd: Command| {
        if ui.button(label).clicked() {
            emit(cmd);
            ui.close();
        }
    };
    item(ui, crate::tr("ℹ  Task Information…"), Command::TaskInfo);
    item(ui, crate::tr("📝  Notes…"), Command::TaskNotes);
    item(ui, crate::tr("🌐  Hyperlink…"), Command::Hyperlink);
    item(ui, crate::tr("🌐  Open hyperlink"), Command::OpenHyperlink);
    item(ui, crate::tr("👥  Assign Resources…"), Command::AssignResources);
    ui.separator();
    item(ui, crate::tr("➕  Insert Task"), Command::InsertTask);
    item(ui, crate::tr("♦  Insert Milestone"), Command::InsertMilestone);
    item(ui, crate::tr("↻  Insert Recurring Task…"), Command::RecurringTask);
    item(ui, crate::tr("🗑  Delete Task"), Command::DeleteSelection);
    ui.separator();
    item(ui, crate::tr("➡  Indent"), Command::Indent);
    item(ui, crate::tr("⬅  Outdent"), Command::Outdent);
    item(ui, crate::tr("🔗  Link Tasks"), Command::LinkSelected);
    item(ui, crate::tr("⛓  Unlink Tasks"), Command::UnlinkSelected);
    ui.separator();
    item(ui, crate::tr("✔  Mark 100% Complete"), Command::SetPercent(100));
    item(ui, crate::tr("📅  Add to Timeline"), Command::ToggleTimeline);
    item(ui, crate::tr("⏸  Inactivate / Activate"), Command::ToggleActive);
    ui.separator();
    item(ui, crate::tr("✂  Cut"), Command::Cut);
    item(ui, crate::tr("📋  Copy"), Command::Copy);
    item(ui, crate::tr("📥  Paste"), Command::Paste);
}
