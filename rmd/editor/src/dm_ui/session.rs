use prefab::PrefabInstanceId;

use crate::document::DocumentId;
use crate::{bake::session::report_bake_output, session::Session};

impl Session {
    pub(crate) fn dm_ui(&mut self, dockspace: u32, feedback: crate::bake::UiFeedback) -> crate::bake::UiFrame {
        let Some(id) = self.state.active() else {
            return crate::bake::UiFrame::default();
        };
        let Some(program) = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
        else {
            return crate::bake::UiFrame::default();
        };
        let target = self
            .state
            .document(id)
            .and_then(|document| document.picked_instance())
            .map(PrefabInstanceId::get);
        let Some(bake) = self.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()) else {
            return crate::bake::UiFrame::default();
        };

        let mut replayed = feedback.clone();
        replayed.mouse_popup_requested = false;
        let drawn = bake.ui(&program.tree, &program.module, target, dockspace, feedback);
        report_bake_output(bake);
        let is_faulted = bake.ui_fault.is_some();

        let frame = match drawn {
            Ok(frame) => {
                if !is_faulted {
                    self.logged_ui_fault = None;
                }

                if frame.committed {
                    if let Some(is_enabled) = frame.modular_preview {
                        self.set_modules_enabled(is_enabled);
                    }

                    let mut settled = vec![id];
                    for other in self.rendered_document_ids() {
                        if other != id && self.replay_ui(other, &replayed) {
                            settled.push(other);
                        }
                    }

                    self.ui_feedback = Some(replayed);
                    for document in settled {
                        self.settle_modules(document, Vec::new());
                    }
                }

                frame
            },
            Err(fault) => {
                let key = fault.key();
                if self.logged_ui_fault.as_ref() != Some(&key) {
                    if let Some(environment) = &self.state.environment {
                        log::warn!("DM ui(): {}", environment.fault_report(&fault));
                    }
                    self.logged_ui_fault = Some(key);
                }

                crate::bake::UiFrame::default()
            },
        };

        self.log_runtimes();
        frame
    }

    pub(crate) fn replay_ui(&mut self, id: DocumentId, feedback: &crate::bake::UiFeedback) -> bool {
        let Some(environment) = self.state.environment.clone() else {
            return false;
        };
        let Some(program) = environment.bake_program.as_ref() else {
            return false;
        };
        let target = self
            .state
            .document(id)
            .and_then(|document| document.picked_instance())
            .map(PrefabInstanceId::get);
        let Some(cache) = self.caches.get_mut(&id) else {
            return false;
        };

        // a bake still on the baker thread is caught up by poll_bake instead
        let Some(bake) = cache.bake.as_mut() else {
            return false;
        };

        let drawn = bake.ui(&program.tree, &program.module, target, 0, feedback.clone());
        report_bake_output(bake);

        let Ok(frame) = drawn else {
            return false;
        };

        cache.pending_rebake.merge(frame.rebake);

        frame.committed
    }

    pub(crate) fn flush_pending_rebake(&mut self, id: DocumentId) {
        let request = self
            .caches
            .get_mut(&id)
            .map(|cache| std::mem::take(&mut cache.pending_rebake))
            .unwrap_or_default();

        if !request.is_empty() {
            self.run_rebake(id, request);
        }
    }

    fn flush_pending_rebakes(&mut self) {
        for id in self.rendered_document_ids() {
            self.flush_pending_rebake(id);
        }
    }

    pub(crate) fn dm_ui_rebake(&mut self, request: crate::bake::UiRebake) {
        if let Some(id) = self.state.active() {
            self.caches.entry(id).or_default().pending_rebake.merge(request);
        }

        self.flush_pending_rebakes();
    }

    fn run_rebake(&mut self, id: DocumentId, request: crate::bake::UiRebake) {
        let Some(program) = self
            .state
            .environment
            .as_ref()
            .and_then(|environment| environment.bake_program.as_ref())
        else {
            return;
        };
        let Some(bake) = self.caches.get_mut(&id).and_then(|cache| cache.bake.as_mut()) else {
            return;
        };

        let update = bake.rebake(&program.tree, &program.module, request);
        report_bake_output(bake);

        self.collect_bake_diagnostics();
        self.apply_bake_update(
            id,
            crate::bake::BakeUpdate {
                appearances: update
                    .appearances
                    .into_iter()
                    .filter_map(PrefabInstanceId::from_raw)
                    .collect(),
                lighting: update.lighting,
            },
        );
        self.settle_modules(id, Vec::new());
    }
}

#[cfg(test)]
mod tests;
