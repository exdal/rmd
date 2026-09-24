use dear_imgui_rs::{Condition, StyleColor, Ui, WindowFlags};
use dmm::Coord;
use editor::{
    document::DocumentId,
    git::{CommitInfo, WebLinks},
    icons::materialdesignicons::ICON_CIRCLE_SMALL,
};

use super::{OverlayRect, git::commit_summary};

pub(super) struct BlamePopup {
    pub(super) coord: Coord,
    pub(super) position: [f32; 2],
    pub(super) bounds: Option<OverlayRect>,
}

pub(super) fn blame_tooltip_position(ui: &Ui, mouse: [f32; 2]) -> [f32; 2] {
    let display = ui.io().display_size();
    let width = 390.0;
    let height = ui.text_line_height_with_spacing() * 6.0;
    [
        if mouse[0] + width + 12.0 > display[0] {
            (mouse[0] - width - 12.0).max(0.0)
        } else {
            mouse[0] + 12.0
        },
        if mouse[1] + height + 12.0 > display[1] {
            (mouse[1] - height - 12.0).max(0.0)
        } else {
            mouse[1] + 12.0
        },
    ]
}

pub(super) fn draw_blame_popup(
    ui: &Ui, document: DocumentId, popup: &BlamePopup, commit: &CommitInfo, web: Option<&WebLinks>,
) -> Option<(OverlayRect, bool)> {
    const WIDTH: f32 = 390.0;
    let flags = WindowFlags::NO_DECORATION
        | WindowFlags::NO_MOVE
        | WindowFlags::NO_SAVED_SETTINGS
        | WindowFlags::NO_DOCKING
        | WindowFlags::NO_FOCUS_ON_APPEARING
        | WindowFlags::ALWAYS_AUTO_RESIZE;
    let _background = ui.push_style_color(StyleColor::WindowBg, ui.clone_style().color(StyleColor::PopupBg));
    ui.window(format!("Blame##popup-{}", document.get()))
        .flags(flags)
        .position(popup.position, Condition::Always)
        .size_constraints([0.0, 0.0], [WIDTH, f32::MAX])
        .build(|| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs() as i64);
            if let Some(web) = web {
                let url = web.commit(&commit.hash);
                ui.text_link_open_url(&commit.short, &url);
            } else {
                ui.text(&commit.short);
                ui.set_item_tooltip("No web remote is configured for this repository");
            }
            ui.same_line();
            ui.text(format!(
                "{ICON_CIRCLE_SMALL} {} {ICON_CIRCLE_SMALL} {}",
                commit.author,
                editor::blame::relative_time(now, commit.time)
            ));
            let padding = ui.clone_style().window_padding()[0];
            commit_summary(ui, commit, web, WIDTH - padding * 2.0);
            let close = ui.small_button("Close");
            let min = ui.window_pos();
            let size = ui.window_size();
            (
                OverlayRect {
                    min,
                    max: [min[0] + size[0], min[1] + size[1]],
                },
                close,
            )
        })
}
