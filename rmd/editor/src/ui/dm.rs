use std::collections::HashSet;

use dear_imgui_rs::{Condition, Id, TreeNodeFlags, Ui};
use editor::bake::{UiCommand, UiFeedback, UiPopupId, UiRebake, UiValue};

use crate::session::Session;

#[derive(Debug, Default)]
pub(crate) struct DmUi {
    feedback: UiFeedback,
    clicks: HashSet<String>,
    pending: bool,
    mouse_popup_requested: bool,
    rebake: UiRebake,
}

impl DmUi {
    pub(crate) fn request_mouse_popup(&mut self) { self.mouse_popup_requested = true; }

    pub(crate) fn draw(&mut self, ui: &Ui, session: &mut Session, dockspace: u32) {
        let feedback = UiFeedback {
            values: self.feedback.values.clone(),
            closed: self.feedback.closed.clone(),
            open_popups: self.feedback.open_popups.clone(),
            clicks: std::mem::take(&mut self.clicks),
            mouse_popup_requested: std::mem::take(&mut self.mouse_popup_requested),
            interacted: std::mem::take(&mut self.pending),
        };

        let frame = session.dm_ui(dockspace, feedback);
        self.draw_commands(ui, &frame.commands, 0);
        self.prune(&frame.commands);

        self.rebake.merge(frame.rebake);
        if !self.rebake.is_empty() && !ui.is_any_item_active() {
            session.dm_ui_rebake(std::mem::take(&mut self.rebake));
        }
    }

    fn draw_commands(&mut self, ui: &Ui, commands: &[UiCommand], from: usize) -> usize {
        let mut index = from;
        while let Some(command) = commands.get(index) {
            index += 1;
            match command {
                UiCommand::End | UiCommand::TreeEnd | UiCommand::EndPopup => return index,
                UiCommand::OpenPopup(popup) => {
                    ui.open_popup(popup_name(*popup));
                },
                UiCommand::BeginPopup(popup) => {
                    let token = ui.begin_popup(popup_name(*popup));
                    self.set_popup_open(*popup, token.is_some());
                    index = match token {
                        Some(token) => {
                            let after = self.draw_commands(ui, commands, index);
                            drop(token);

                            after
                        },
                        None => skip(commands, index, &UiCommand::EndPopup),
                    };
                },
                UiCommand::Begin { key, label, dock, size } => {
                    if let Some(dock) = dock {
                        ui.set_next_window_dock_id_with_cond(Id::from(*dock), Condition::FirstUseEver);
                    }

                    let mut window = ui.window(format!("{label}###{key}"));
                    if let Some(size) = size {
                        window = window.size(*size, Condition::FirstUseEver);
                    }

                    let mut after = index;
                    let drawn = window.build(|| after = self.draw_commands(ui, commands, index));
                    self.set_open(key, drawn.is_some());
                    index = match drawn {
                        Some(()) => after,
                        None => skip(commands, index, &UiCommand::End),
                    };
                },
                UiCommand::Tree { key, label } => {
                    let node = ui.tree_node(format!("{label}##{key}"));
                    self.set_open(key, node.is_some());
                    index = match node {
                        Some(node) => {
                            let after = self.draw_commands(ui, commands, index);
                            drop(node);

                            after
                        },
                        None => skip(commands, index, &UiCommand::TreeEnd),
                    };
                },
                UiCommand::CollapsingHeader { key, label } => {
                    let open = ui.collapsing_header(format!("{label}##{key}"), TreeNodeFlags::empty());
                    self.set_open(key, open);
                },
                UiCommand::Text { text, color } => match color {
                    Some([r, g, b]) => ui.text_colored([*r, *g, *b, 1.0], text),
                    None => ui.text(text),
                },
                UiCommand::Button { key, label } => {
                    if ui.button(format!("{label}##{key}")) {
                        self.press(key);
                    }
                },
                UiCommand::Radio { key, label, active } => {
                    if ui.radio_button(format!("{label}##{key}"), *active) {
                        self.press(key);
                    }
                },
                UiCommand::Checkbox { key, label, value } => {
                    let mut edited = *value;
                    if ui.checkbox(format!("{label}##{key}"), &mut edited) {
                        self.set_value(key, UiValue::Bool(edited));
                    }
                },
                UiCommand::Slider {
                    key,
                    label,
                    value,
                    range,
                } => {
                    let mut edited = *value;
                    if ui.slider(format!("{label}##{key}"), range[0], range[1], &mut edited) {
                        self.set_value(key, UiValue::Num(edited));
                    }
                },
                UiCommand::Drag {
                    key,
                    label,
                    value,
                    speed,
                    range,
                } => {
                    let mut edited = *value;
                    if ui
                        .drag_config(format!("{label}##{key}"))
                        .speed(*speed)
                        .range(range[0], range[1])
                        .build(ui, &mut edited)
                    {
                        self.set_value(key, UiValue::Num(edited));
                    }
                },
                UiCommand::InputText { key, label, value } => {
                    let mut edited = value.clone();
                    if ui.input_text(format!("{label}##{key}"), &mut edited).build() {
                        self.set_value(key, UiValue::Text(edited));
                    }
                },
                UiCommand::Separator { label } => match label {
                    Some(label) => ui.separator_with_text(label),
                    None => ui.separator(),
                },
                UiCommand::SameLine => ui.same_line(),
            }
        }

        index
    }

    fn press(&mut self, key: &str) {
        self.clicks.insert(key.to_owned());
        self.pending = true;
    }

    fn set_value(&mut self, key: &str, value: UiValue) {
        if self.feedback.values.get(key) != Some(&value) {
            self.feedback.values.insert(key.to_owned(), value);
            self.pending = true;
        }
    }

    fn set_open(&mut self, key: &str, open: bool) {
        match open {
            true => self.feedback.closed.remove(key),
            false => self.feedback.closed.insert(key.to_owned()),
        };
    }

    fn set_popup_open(&mut self, popup: UiPopupId, open: bool) {
        match open {
            true => self.feedback.open_popups.insert(popup),
            false => self.feedback.open_popups.remove(&popup),
        };
    }

    fn prune(&mut self, commands: &[UiCommand]) {
        let mut drawn = HashSet::new();
        let mut popups = HashSet::new();
        for command in commands {
            if let Some(key) = key_of(command) {
                drawn.insert(key);
            }
            if let UiCommand::BeginPopup(popup) = command {
                popups.insert(*popup);
            }
        }

        self.feedback.values.retain(|key, _| drawn.contains(key.as_str()));
        self.feedback.closed.retain(|key| drawn.contains(key.as_str()));
        self.feedback.open_popups.retain(|popup| popups.contains(popup));
        self.clicks.retain(|key| drawn.contains(key.as_str()));
    }
}

fn popup_name(popup: UiPopupId) -> &'static str {
    match popup {
        UiPopupId::Mouse => "demir-mouse-popup",
    }
}

fn skip(commands: &[UiCommand], from: usize, terminator: &UiCommand) -> usize {
    let opener = match terminator {
        UiCommand::TreeEnd => |command: &UiCommand| matches!(command, UiCommand::Tree { .. }),
        UiCommand::EndPopup => |command: &UiCommand| matches!(command, UiCommand::BeginPopup(_)),
        _ => |command: &UiCommand| matches!(command, UiCommand::Begin { .. }),
    };

    let mut depth = 0usize;
    for (offset, command) in commands.iter().enumerate().skip(from) {
        if opener(command) {
            depth += 1;
        } else if command == terminator {
            match depth {
                0 => return offset + 1,
                _ => depth -= 1,
            }
        }
    }

    commands.len()
}

fn key_of(command: &UiCommand) -> Option<&str> {
    match command {
        UiCommand::Begin { key, .. }
        | UiCommand::Button { key, .. }
        | UiCommand::Radio { key, .. }
        | UiCommand::Checkbox { key, .. }
        | UiCommand::Slider { key, .. }
        | UiCommand::Drag { key, .. }
        | UiCommand::InputText { key, .. }
        | UiCommand::Tree { key, .. }
        | UiCommand::CollapsingHeader { key, .. } => Some(key),
        UiCommand::End
        | UiCommand::TreeEnd
        | UiCommand::OpenPopup(_)
        | UiCommand::BeginPopup(_)
        | UiCommand::EndPopup
        | UiCommand::Text { .. }
        | UiCommand::Separator { .. }
        | UiCommand::SameLine => None,
    }
}

#[cfg(test)]
impl DmUi {
    pub(crate) fn replay(&mut self, ui: &Ui, commands: &[UiCommand]) {
        self.draw_commands(ui, commands, 0);
        self.prune(commands);
    }

    pub(crate) fn values(&self) -> &std::collections::HashMap<String, UiValue> { &self.feedback.values }

    pub(crate) fn closed(&self) -> &HashSet<String> { &self.feedback.closed }

    pub(crate) fn clicks(&self) -> &HashSet<String> { &self.clicks }

    pub(crate) fn open_popups(&self) -> &HashSet<UiPopupId> { &self.feedback.open_popups }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn context() -> dear_imgui_rs::Context {
        let mut context = dear_imgui_rs::Context::create();
        context.set_ini_filename(None::<PathBuf>).unwrap();
        context.font_atlas().try_claim_legacy_renderer().unwrap().build();
        context.io_mut().set_display_size([800.0, 600.0]);
        context.io_mut().set_delta_time(1.0 / 60.0);
        context
    }

    fn replay(commands: &[UiCommand]) -> DmUi {
        let _guard = super::super::IMGUI_CONTEXT
            .lock()
            .unwrap_or_else(|guard| guard.into_inner());
        let mut context = context();
        let mut panel = DmUi::default();
        {
            let ui = context.frame();
            panel.replay(ui, commands);
        }
        assert!(context.render_legacy().valid());

        panel
    }

    fn begin(key: &str) -> UiCommand {
        UiCommand::Begin {
            key: String::from(key),
            label: String::from(key),
            dock: None,
            size: None,
        }
    }

    fn tree(key: &str) -> UiCommand {
        UiCommand::Tree {
            key: String::from(key),
            label: String::from(key),
        }
    }

    fn header(key: &str) -> UiCommand {
        UiCommand::CollapsingHeader {
            key: String::from(key),
            label: String::from(key),
        }
    }

    fn radio(key: &str, active: bool) -> UiCommand {
        UiCommand::Radio {
            key: String::from(key),
            label: String::from(key),
            active,
        }
    }

    fn checkbox(key: &str) -> UiCommand {
        UiCommand::Checkbox {
            key: String::from(key),
            label: String::from(key),
            value: false,
        }
    }

    #[test]
    fn a_replayed_stream_records_which_nodes_are_collapsed() {
        let panel = replay(&[
            begin("W"),
            tree("W/T"),
            UiCommand::Text {
                text: String::from("inside"),
                color: None,
            },
            UiCommand::TreeEnd,
            header("W/H"),
            checkbox("W/C"),
            radio("W/R1", true),
            radio("W/R2", false),
            UiCommand::End,
        ]);

        assert_eq!(
            panel.closed(),
            &HashSet::from([String::from("W/T"), String::from("W/H")]),
            "nodes start collapsed in imgui, and the window itself drew",
        );
    }

    #[test]
    fn an_unbalanced_stream_draws_without_panicking() {
        // The VM closes what a profile leaves open, so this only guards the replay itself.
        let panel = replay(&[begin("W"), tree("W/T"), header("W/H")]);

        assert!(panel.closed().contains("W/T"));
    }

    #[test]
    fn a_widget_the_profile_stopped_drawing_is_forgotten() {
        let _guard = super::super::IMGUI_CONTEXT
            .lock()
            .unwrap_or_else(|guard| guard.into_inner());
        let mut context = context();
        let mut panel = DmUi::default();
        panel
            .feedback
            .values
            .insert(String::from("W/gone"), UiValue::Bool(true));
        panel.feedback.closed.insert(String::from("W/also-gone"));
        panel.clicks.insert(String::from("W/pressed"));

        {
            let ui = context.frame();
            panel.replay(ui, &[begin("W"), checkbox("W/C"), UiCommand::End]);
        }
        assert!(context.render_legacy().valid());

        assert!(panel.values().is_empty());
        assert_eq!(panel.closed(), &HashSet::new());
        assert!(panel.clicks().is_empty());
    }

    #[test]
    fn a_mouse_popup_opens_only_when_the_stream_requests_it() {
        let popup = UiPopupId::Mouse;
        let open = replay(&[
            UiCommand::OpenPopup(popup),
            UiCommand::BeginPopup(popup),
            UiCommand::Text {
                text: String::from("inside"),
                color: None,
            },
            UiCommand::EndPopup,
        ]);
        assert_eq!(open.open_popups(), &HashSet::from([popup]));

        let closed = replay(&[
            UiCommand::BeginPopup(popup),
            UiCommand::Text {
                text: String::from("not drawn"),
                color: None,
            },
            UiCommand::EndPopup,
        ]);
        assert!(closed.open_popups().is_empty());
    }
}
