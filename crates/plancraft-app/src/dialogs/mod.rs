//! Modal-style windows.

pub mod extra;
pub mod mcp_panel;
pub mod misc;
pub mod project_info;
pub mod reports;
pub mod resource_info;
pub mod task_info;
pub mod working_time;

/// Shared title and frame for the application's dialog windows.
fn window(ctx: &egui::Context, title: impl Into<String>) -> egui::Window<'static> {
    let frame = egui::Frame::window(&ctx.style_of(ctx.theme())).inner_margin(egui::Margin::symmetric(10, 4));
    egui::Window::new(egui::RichText::new(title.into()).size(16.0).strong()).frame(frame).collapsible(false)
}

fn tabs<T: Copy + PartialEq>(ui: &mut egui::Ui, current: &mut T, tabs: &[(T, &str)]) {
    let mut selected_rect = None;
    let row = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for &(tab, label) in tabs {
            let selected = *current == tab;
            let button = egui::Button::new(egui::RichText::new(label).color(if selected {
                crate::theme::pal().ACCENT_DARK
            } else {
                crate::theme::pal().TEXT
            }))
            .fill(if selected { crate::theme::pal().BG } else { crate::theme::pal().PANEL_BG })
            .stroke(egui::Stroke::new(1.0, crate::theme::pal().GRID_LINE_STRONG))
            .corner_radius(egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 });
            let response = ui.add_sized([0.0, 29.0], button);
            if selected {
                selected_rect = Some(response.rect);
            }
            if response.clicked() {
                *current = tab;
            }
        }
    });
    let y = row.response.rect.bottom();
    ui.painter().hline(ui.min_rect().left()..=ui.max_rect().right(), y, egui::Stroke::new(1.0, crate::theme::pal().GRID_LINE_STRONG));
    if let Some(rect) = selected_rect {
        ui.painter().hline(rect.left() + 1.0..=rect.right() - 1.0, y, egui::Stroke::new(2.0, crate::theme::pal().BG));
    }
    ui.add_space(4.0);
}

#[derive(Default)]
pub struct Dialogs {
    pub task_info: Option<task_info::TaskInfo>,
    pub resource_info: Option<resource_info::ResourceInfo>,
    pub project_info: Option<project_info::ProjectInfo>,
    pub working_time: Option<working_time::WorkingTime>,
    pub assign: Option<misc::AssignResources>,
    pub external: Option<misc::ExternalLinks>,
    pub recurring: Option<extra::RecurringTask>,
    pub baseline: Option<extra::BaselineDialog>,
    pub update_project: Option<extra::UpdateProjectDialog>,
    pub wbs: Option<extra::WbsDialog>,
    pub hyperlink: Option<extra::HyperlinkDialog>,
    pub reports: Option<reports::ReportsWindow>,
    pub templates: bool,
    pub stats: bool,
    pub about: bool,
    pub history: bool,
    pub mcp: bool,
}
