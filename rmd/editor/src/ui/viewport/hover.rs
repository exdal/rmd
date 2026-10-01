use dear_imgui_rs::{MouseButton, Ui};
use dmm::Coord;
use editor::{
    blame::{BlameCell, pending_note, relative_time},
    document::DocumentId,
    icons::materialdesignicons::ICON_CIRCLE_SMALL,
};

use super::ViewFrame;
use crate::{
    session::Session,
    ui::{BlamePopup, blame_tooltip_position, draw_blame_popup, draw_conflict_tooltip, draw_diff_tooltip},
};

pub(super) fn draw_git_hover(ui: &Ui, session: &Session, frame: &mut ViewFrame<'_>, is_over_blame_popup: bool) -> bool {
    let id = frame.id;
    let hovered_conflict = frame.coord.and_then(|coord| {
        session
            .git_state(id)
            .and_then(|git| git.conflicts.as_ref())
            .and_then(|state| state.conflict_at(coord))
    });
    let hovered_diff = frame
        .coord
        .filter(|_| hovered_conflict.is_none() && session.git_state(id).is_some_and(|git| git.show_diff))
        .and_then(|coord| session.diff_at(id, coord).map(|change| (coord, change)));
    let show_blame = session.git_state(id).is_some_and(|git| git.show_blame);
    let hovered_blame = frame
        .coord
        .filter(|_| show_blame && hovered_conflict.is_none() && hovered_diff.is_none())
        .and_then(|coord| session.blame_at(id, coord).map(|cell| (coord, cell)));
    let hovered_commit = match hovered_blame {
        Some((coord, (BlameCell::Commit(..), false))) => Some(coord),
        _ => None,
    };
    let mut tooltip_position = None;
    if frame.blame_popup.is_none() {
        if let (Some(coord), Some(conflict)) = (frame.coord, hovered_conflict) {
            ui.tooltip(|| draw_conflict_tooltip(ui, coord, conflict));
        } else if let Some((coord, (kind, before, after))) = hovered_diff
            && let Some(state) = session.git_state(id).and_then(|git| git.diff.as_ref())
        {
            let (from, to) = (state.from.label(), state.to.label());
            ui.tooltip(|| draw_diff_tooltip(ui, coord, kind, (&from, before), (&to, after)));
        } else if let Some((_, (cell, changed))) = hovered_blame {
            tooltip_position = draw_blame_tooltip(ui, session, id, cell, changed);
        }
    }

    if !show_blame {
        *frame.blame_popup = None;
    }

    update_blame_popup(
        ui,
        session,
        frame,
        hovered_commit,
        tooltip_position,
        is_over_blame_popup,
    )
}

fn draw_blame_tooltip(
    ui: &Ui, session: &Session, id: DocumentId, cell: BlameCell<'_>, changed: bool,
) -> Option<[f32; 2]> {
    let mut position = None;
    match cell {
        BlameCell::Commit(_, commit) if !changed => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |time| time.as_secs() as i64);
            let detail = format!(
                "{} {ICON_CIRCLE_SMALL} {} {ICON_CIRCLE_SMALL} {}\n{}\n",
                commit.short,
                commit.author,
                relative_time(now, commit.time),
                commit.summary
            );
            ui.tooltip(|| {
                ui.text(detail);
                position = Some(ui.window_pos());
            });
        },
        BlameCell::Boundary if !changed => {
            if let Some(label) = session
                .git_state(id)
                .and_then(|git| git.blame.as_ref())
                .map(|blame| blame.result.boundary_label())
            {
                ui.tooltip_text(label);
            }
        },
        _ => {
            if let Some(note) = pending_note(cell, changed) {
                ui.tooltip_text(note);
            }
        },
    }

    position
}

fn update_blame_popup(
    ui: &Ui, session: &Session, frame: &mut ViewFrame<'_>, hovered_commit: Option<Coord>,
    tooltip_position: Option<[f32; 2]>, is_over_blame_popup: bool,
) -> bool {
    let blame_popup = &mut *frame.blame_popup;
    let is_blame_clicked = ui.is_mouse_clicked(MouseButton::Left) && hovered_commit.is_some();
    if let Some(coord) = hovered_commit.filter(|_| is_blame_clicked) {
        let position = blame_popup
            .as_ref()
            .filter(|popup| popup.coord == coord)
            .map(|popup| popup.position)
            .or(tooltip_position)
            .unwrap_or_else(|| blame_tooltip_position(ui, frame.mouse));
        *blame_popup = Some(BlamePopup {
            coord,
            position,
            bounds: None,
        });
    } else if ui.is_mouse_clicked(MouseButton::Left) && !is_over_blame_popup {
        *blame_popup = None;
    }

    if let Some(popup) = blame_popup.as_mut() {
        let commit = match session.blame_at(frame.id, popup.coord) {
            Some((BlameCell::Commit(_, commit), false)) => Some(commit),
            _ => None,
        };

        if let Some(commit) = commit {
            let web = session.git_state(frame.id).and_then(|git| git.web.as_ref());
            if let Some((bounds, close)) = draw_blame_popup(ui, frame.id, popup, commit, web) {
                popup.bounds = Some(bounds);
                if close {
                    *blame_popup = None;
                }
            }
        } else {
            *blame_popup = None;
        }
    }

    is_blame_clicked || is_over_blame_popup
}
