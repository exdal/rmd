use dear_imgui_rs::{Key, Ui};
use net::{Comment, CommentId, MAX_COMMENT_LEN};
use ui::{icons::materialdesignicons::ICON_CLOSE, widgets::dpi};

use super::{PENDING_COLOR, peer_color};
use crate::{
    coop::Coop,
    document::DocumentId,
    frontend::dialog::SAVE_ERROR_COLOR,
    session::Session,
    viewport::{camera::Controller, panel::overlay::OverlayRect},
};

pub(crate) const COMMENT_POPUP: &str = "##coop-comment";

const COMMENT_WIDTH: f32 = 240.0;

const COMMENT_HEADER: f32 = 5.0;

const COMMENT_BG: [f32; 4] = [0.1, 0.1, 0.12, 0.94];

pub(crate) struct CommentDraft {
    pub(crate) document: DocumentId,
    pos: [f32; 2],
    text: String,
}

impl CommentDraft {
    pub(crate) const fn new(document: DocumentId, pos: [f32; 2]) -> Self {
        Self {
            document,
            pos,
            text: String::new(),
        }
    }
}

pub(crate) fn draw_comment_composer(ui: &Ui, session: &Session, draft: &mut Option<CommentDraft>) {
    let Some(state) = draft.as_mut() else {
        return;
    };

    let Some(_popup) = ui.begin_popup(COMMENT_POPUP) else {
        *draft = None;
        return;
    };

    if ui.is_window_appearing() {
        ui.set_keyboard_focus_here();
    }

    ui.set_next_item_width(COMMENT_WIDTH * dpi(ui));
    let submitted = ui
        .input_text("##comment-text", &mut state.text)
        .hint("Add a comment")
        .enter_returns_true(true)
        .build();

    let text = state.text.trim();
    if submitted && !text.is_empty() {
        session.add_coop_comment(state.document, state.pos, text.chars().take(MAX_COMMENT_LEN).collect());
    }

    if submitted || ui.is_key_pressed(Key::Escape) {
        ui.close_current_popup();
        *draft = None;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CommentHit {
    pub(crate) id: CommentId,
    pub(crate) delete: bool,
}

struct CommentLayout<'a> {
    comment: &'a Comment,
    // the author may have left
    nick: Option<&'a str>,
    min: [f32; 2],
    max: [f32; 2],
    row: [f32; 2],
    text: [f32; 2],
    delete: [f32; 2],
    delete_size: [f32; 2],
}

impl CommentLayout<'_> {
    fn contains(&self, point: [f32; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&point[0]) && (self.min[1]..=self.max[1]).contains(&point[1])
    }

    fn delete_contains(&self, point: [f32; 2]) -> bool {
        let max = [
            self.delete[0] + self.delete_size[0],
            self.delete[1] + self.delete_size[1],
        ];

        (self.delete[0]..=max[0]).contains(&point[0]) && (self.delete[1]..=max[1]).contains(&point[1])
    }
}

fn comment_layouts<'a>(
    ui: &'a Ui, camera: &'a Controller, viewport_min: [f32; 2], coop: &'a Coop, map: &'a str, z: u32,
) -> impl Iterator<Item = CommentLayout<'a>> + 'a {
    let scale = dpi(ui);
    let padding = 6.0 * scale;
    let wrap = COMMENT_WIDTH * scale;
    let header = COMMENT_HEADER * scale;
    let delete_size = ui.calc_text_size(ICON_CLOSE.to_string());

    coop.comments
        .values()
        .filter(move |comment| comment.map == map && comment.z == z)
        .filter_map(move |comment| {
            let local = camera.map_to_screen(comment.pos);
            if !local.iter().all(|value| value.is_finite()) {
                return None;
            }

            let nick = coop.nick_of(comment.author);
            let nick_width = nick.map_or(0.0, |nick| ui.calc_text_size(nick)[0]);
            let text_size = ui.calc_text_size_with_opts(&comment.text, false, wrap);
            let width = (nick_width + padding + delete_size[0]).max(text_size[0]);
            let min = [viewport_min[0] + local[0], viewport_min[1] + local[1]];
            let max = [
                min[0] + width + padding * 2.0,
                min[1] + header + delete_size[1] + text_size[1] + padding * 2.5,
            ];

            let row = [min[0] + padding, min[1] + header + padding];
            Some(CommentLayout {
                comment,
                nick,
                min,
                max,
                row,
                text: [row[0], row[1] + delete_size[1] + padding * 0.5],
                delete: [max[0] - padding - delete_size[0], row[1]],
                delete_size,
            })
        })
}

pub(crate) fn comment_at(
    ui: &Ui, camera: &Controller, viewport_min: [f32; 2], coop: &Coop, map: &str, z: u32, point: [f32; 2],
) -> Option<CommentHit> {
    comment_layouts(ui, camera, viewport_min, coop, map, z)
        .filter(|layout| layout.contains(point))
        .last()
        .map(|layout| CommentHit {
            id: layout.comment.id,
            delete: layout.delete_contains(point),
        })
}

pub(crate) fn draw_comments(
    ui: &Ui, camera: &Controller, viewport: OverlayRect, coop: &Coop, map: &str, z: u32, hovered: Option<CommentHit>,
) {
    let header = COMMENT_HEADER * dpi(ui);
    let wrap = COMMENT_WIDTH * dpi(ui);
    let draw = ui.get_window_draw_list();
    draw.with_clip_rect(viewport.min, viewport.max, || {
        for layout in comment_layouts(ui, camera, viewport.min, coop, map, z) {
            let color = peer_color(layout.comment.author);
            draw.add_rect(layout.min, layout.max, COMMENT_BG).filled(true).build();
            draw.add_rect(layout.min, [layout.max[0], layout.min[1] + header], color)
                .filled(true)
                .build();

            if let Some(nick) = layout.nick {
                draw.add_text(layout.row, color, nick);
            }

            if let Some(hit) = hovered.filter(|hit| hit.id == layout.comment.id) {
                let icon = if hit.delete { SAVE_ERROR_COLOR } else { PENDING_COLOR };
                draw.add_text(layout.delete, icon, ICON_CLOSE.to_string());
            }

            draw.add_text_with_font(
                ui.current_font(),
                ui.current_font_size(),
                layout.text,
                [1.0; 4],
                &layout.comment.text,
                wrap,
                None,
            );
        }
    });
}
