use dear_imgui_rs::{Ui, WindowKey, WindowKeyError};
use dmm::Coord;
use editor::{conflict::Side, document::DocumentId};

use crate::{session::Session, settings::Settings};

#[derive(Default)]
pub(super) struct GitPanelOutput {
    pub center: Option<(DocumentId, Coord)>,
    pub load_conflicts: Option<DocumentId>,
}

pub(super) struct GitPanel {
    window: WindowKey,
    pub open: bool,
}

impl GitPanel {
    pub fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("git-panel", "Git")?,
            open: false,
        })
    }

    pub fn draw(&mut self, ui: &Ui, session: &mut Session, settings: &Settings) -> GitPanelOutput {
        let mut output = GitPanelOutput::default();
        if !self.open {
            return output;
        }
        ui.window(&self.window).opened(&mut self.open).build(|| {
            if !settings.git_enabled {
                ui.text_disabled("Git integration is disabled in settings");
                return;
            }
            let Some(id) = session.state.active() else {
                ui.text_disabled("No map selected");
                return;
            };
            let Some(git) = session.git_state(id) else {
                ui.text_disabled("This map is outside a Git worktree");
                return;
            };
            let branch = git
                .branch
                .clone()
                .unwrap_or_else(|| String::from("Reading repository..."));
            let head = git.head.as_ref().map(|head| head.short.clone());
            let operation = git
                .operation
                .as_ref()
                .map(|operation| format!("{} {}", operation.kind.label(), operation.theirs.describe()));
            let pending_load = git.pending_load;
            let unmerged = git.unmerged_on_disk;
            let staging = git.staging;
            let error = git.error.clone();
            let has_loaded_conflicts = git.conflicts.is_some();
            let total = git.conflicts.as_ref().map_or(0, |conflicts| conflicts.len());
            let remaining = session.unresolved_conflicts(id);
            let theirs = git.conflicts.as_ref().map_or(String::from("theirs"), |conflicts| {
                conflicts.side_label(Side::Theirs).to_owned()
            });
            let regions = session.conflict_regions(id, None);
            let saved = session.state.document(id).is_some_and(|document| !document.is_dirty());

            ui.text(format!(
                "{}{}",
                branch,
                head.map_or(String::new(), |head| format!(" ({head})"))
            ));
            if let Some(operation) = operation {
                ui.text(operation);
            }
            ui.separator();
            if pending_load {
                ui.text("Merge conflicts on disk");
                if ui.button("Load conflicts") {
                    output.load_conflicts = Some(id);
                }
            }
            if unmerged || total != 0 {
                ui.text(format!("{remaining} unresolved / {total} conflicts"));
                if total != 0 {
                    let mut take_all = None;
                    if ui.button("Take HEAD for all") {
                        take_all = Some(Side::Ours);
                    }
                    ui.same_line();
                    if ui.button(format!("Take {theirs} for all")) {
                        take_all = Some(Side::Theirs);
                    }
                    if let Some(side) = take_all {
                        let all = session
                            .git_state(id)
                            .and_then(|git| git.conflicts.as_ref())
                            .map(|conflicts| conflicts.coords().to_vec())
                            .unwrap_or_default();
                        session.resolve_conflict(id, &all, side);
                    }
                }
                for (index, region) in regions.iter().enumerate() {
                    if ui.text_link(format!(
                        "Region {}: {}, {}, {} ({} tiles)",
                        index + 1,
                        region.anchor.x,
                        region.anchor.y,
                        region.anchor.z,
                        region.tiles.len()
                    )) {
                        output.center = Some((id, region.anchor));
                    }
                }
                let enabled = unmerged && has_loaded_conflicts && remaining == 0 && saved && !staging;
                if ui.with_disabled_if(!enabled, || {
                    ui.button(if staging { "Staging..." } else { "Mark resolved" })
                }) {
                    session.mark_resolved(id);
                }
            }
            if let Some(error) = error {
                ui.text_colored(super::SAVE_ERROR_COLOR, error);
            }
        });
        output
    }
}
