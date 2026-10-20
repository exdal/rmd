use std::hash::{DefaultHasher, Hash, Hasher};

use dear_imgui_rs::{Condition, StyleColor, TreeNodeFlags, Ui, WindowKey, WindowKeyError};

use super::common::{centered_note, dpi, focus_window_on_hover};
use crate::{
    external_editor::SourceLocation,
    session::{FaultState, RuntimeFault, Session},
    settings::Settings,
};

const RUNTIMES_WINDOW_SIZE: [f32; 2] = [560.0, 320.0];

#[derive(Default)]
pub(super) struct RuntimesOutput {
    pub(super) open_source: Option<SourceLocation>,
    pub(super) copy: Option<String>,
    pub(super) clear: bool,
}

pub(super) struct RuntimesPanel {
    window: WindowKey,
    focus: bool,
}

impl RuntimesPanel {
    pub(super) fn new() -> Result<Self, WindowKeyError> {
        Ok(Self {
            window: WindowKey::new("runtimes", "Runtimes")?,
            focus: false,
        })
    }

    pub(super) fn request(&mut self) {
        self.focus = true;
    }

    pub(super) fn draw(&mut self, ui: &Ui, session: &Session, settings: &Settings, open: &mut bool) -> RuntimesOutput {
        let mut output = RuntimesOutput::default();
        let focus = std::mem::take(&mut self.focus);
        if !*open {
            return output;
        }

        ui.window(&self.window)
            .opened(open)
            .size(RUNTIMES_WINDOW_SIZE.map(|size| size * dpi(ui)), Condition::FirstUseEver)
            .focused(focus)
            .build(|| {
                if settings.focus_windows_on_hover {
                    focus_window_on_hover(ui);
                }

                let maps = session.runtime_faults();
                if maps.is_empty() {
                    centered_note(ui, "No runtimes.");
                    return;
                }

                output.clear = ui.small_button("Clear");
                ui.same_line();
                ui.text_disabled("Faults that stopped are greyed out.");

                for (position, (document, faults)) in maps.iter().enumerate() {
                    let title = session
                        .state
                        .document(*document)
                        .map(|document| document.title())
                        .unwrap_or_default();
                    let _map = ui.push_id(position);
                    if !ui.collapsing_header(&title, TreeNodeFlags::DEFAULT_OPEN) {
                        continue;
                    }

                    let mut previous_hook = None;
                    for fault in faults {
                        let hook = Some(fault.hook);
                        if previous_hook != hook {
                            ui.text_disabled(fault.hook.map_or("setup", |hook| hook.name()));
                            previous_hook = hook;
                        }

                        let _fault = ui.push_id(fault_id(fault));
                        draw_fault(ui, session, fault, &mut output);
                    }
                }
            });

        output
    }
}

fn draw_fault(ui: &Ui, session: &Session, fault: &RuntimeFault<'_>, output: &mut RuntimesOutput) {
    let Some(environment) = session.state.environment.as_deref() else {
        return;
    };

    let program = environment.bake_program.as_ref();
    let proc_name = |proc| program.map_or("<unknown proc>", |program| vm::proc_name(&program.module, proc));
    let location = |location: core::location::Location| {
        location
            .display(environment.relative_bake_file(location.file))
            .to_string()
    };

    let error = &fault.fault;
    let mut label = format!("{}x {}", fault.count, error.kind);
    if let Some(proc) = error.proc() {
        label.push_str(&format!(" in {}", proc_name(proc)));
    }

    if error.location.begin.line > 0 {
        label.push_str(&format!(" at {}", location(error.location)));
    }

    let stopped = (fault.state == FaultState::Stopped)
        .then(|| ui.push_style_color(StyleColor::Text, ui.style_color(StyleColor::TextDisabled)));
    let node = ui.tree_node(format!("{label}###fault"));
    drop(stopped);
    ui.same_line();
    if ui.small_button("Copy") {
        output.copy = Some(environment.fault_fault_report(error));
    }

    let Some(_node) = node else {
        return;
    };

    for frame in &error.frames {
        let text = format!("{} ({})", proc_name(frame.proc), location(frame.location));
        let source = environment.bake_file(frame.location.file).map(|path| SourceLocation {
            path: path.to_path_buf(),
            line: frame.location.begin.line as usize,
            column: frame.location.begin.col as usize,
        });
        match source {
            Some(source) if ui.text_link(&text) => output.open_source = Some(source),
            Some(_) => {},
            None => ui.text(&text),
        }
    }
}

fn fault_id(fault: &RuntimeFault<'_>) -> usize {
    let mut hasher = DefaultHasher::new();
    (fault.hook, fault.fault.key()).hash(&mut hasher);

    hasher.finish() as usize
}

#[cfg(test)]
mod tests {
    use super::RuntimesPanel;
    use crate::{
        session::fixtures::faulting_profile_session,
        settings::Settings,
        ui::{IMGUI_CONTEXT, fixtures::rectangle_context},
    };

    #[test]
    fn the_panel_draws_a_profile_that_faults() {
        let _guard = IMGUI_CONTEXT.lock().unwrap();
        let mut context = rectangle_context();
        let (mut session, _) = faulting_profile_session();
        session.dm_ui(0, editor::bake::UiFeedback::default());
        let mut panel = RuntimesPanel::new().unwrap();
        let settings = Settings::default();
        let mut open = true;

        for _ in 0..2 {
            let ui = context.frame();
            let output = panel.draw(ui, &session, &settings, &mut open);
            assert!(output.open_source.is_none() && output.copy.is_none());
            assert!(context.render_legacy().valid());
        }

        assert!(open);
    }
}
