use core::types::Identifier;
use std::collections::{HashMap, HashSet};

use crate::{FaultKind, GenericValue, Intrinsic, eval::Evaluator, lighting::parse_color};

type Result<T> = std::result::Result<T, crate::Fault>;

const MAX_COMMANDS: usize = 4096;
const MAX_DEPTH: usize = 16;
const MAX_LABEL: usize = 128;
const MAX_TEXT: usize = 1024;

const DOCKSPACE_HANDLE: f32 = 1.0;

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Begin {
        key: String,
        label: String,
        dock: Option<u32>,
        size: Option<[f32; 2]>,
    },
    End,
    Text {
        text: String,
        color: Option<[f32; 3]>,
    },
    Button {
        key: String,
        label: String,
    },
    Checkbox {
        key: String,
        label: String,
        value: bool,
    },
    Radio {
        key: String,
        label: String,
        active: bool,
    },
    Slider {
        key: String,
        label: String,
        value: f32,
        range: [f32; 2],
    },
    Drag {
        key: String,
        label: String,
        value: f32,
        speed: f32,
        range: [f32; 2],
    },
    InputText {
        key: String,
        label: String,
        value: String,
    },
    Separator {
        label: Option<String>,
    },
    SameLine,
    Tree {
        key: String,
        label: String,
    },
    TreeEnd,
    CollapsingHeader {
        key: String,
        label: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Num(f32),
    Text(String),
}

/// Multiframe data is kept here.
#[derive(Debug, Clone, Default)]
pub struct Feedback {
    /// Widget values the viewer has edited. A profile's own value is the one used until then.
    pub values: HashMap<String, Value>,
    /// Buttons pressed since the last frame, read once.
    pub clicks: HashSet<String>,
    /// Windows and nodes the viewer has collapsed. Anything absent counts as open.
    pub closed: HashSet<String>,
    pub interacted: bool,
}

/// Which placements a stage has to be re-derived for. `Some(0)` is every placement, `Some(mask)`
/// only the ones whose `demir_bake_group` shares a bit with it, `None` none at all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Rebake {
    pub appearance: Option<u32>,
    pub light: Option<u32>,
    pub highlight: Option<u32>,
}

const KIND_APPEARANCE: u32 = 1;
const KIND_LIGHT: u32 = 2;
const KIND_HIGHLIGHT: u32 = 4;

impl Rebake {
    pub fn is_empty(self) -> bool { self == Self::default() }

    /// Folds in what another frame asked for, for a caller holding a request back until the viewer
    /// lets go of the widget driving it.
    pub fn merge(&mut self, other: Self) {
        for (mine, theirs) in [
            (&mut self.appearance, other.appearance),
            (&mut self.light, other.light),
            (&mut self.highlight, other.highlight),
        ] {
            *mine = match (*mine, theirs) {
                (Some(0), _) | (_, Some(0)) => Some(0),
                (Some(current), Some(asked)) => Some(current | asked),
                (None, asked) => asked,
                (current, None) => current,
            };
        }
    }

    /// Zero is every placement, so it swallows any mask asked for beside it.
    fn request(&mut self, kinds: u32, groups: u32) {
        self.merge(Self {
            appearance: (kinds & KIND_APPEARANCE != 0).then_some(groups),
            light: (kinds & KIND_LIGHT != 0).then_some(groups),
            highlight: (kinds & KIND_HIGHLIGHT != 0).then_some(groups),
        });
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frame {
    pub commands: Vec<Command>,
    /// Whether the frame kept its heap writes, and so whether the map has to be re-derived.
    pub committed: bool,
    pub rebake: Rebake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Window,
    Tree,
}

#[derive(Debug, Default)]
pub struct Panel {
    commands: Vec<Command>,
    feedback: Feedback,
    active: bool,
    dockspace: u32,
    scope: Vec<(Scope, String)>,
    seen: HashMap<String, u32>,
    next_dock: Option<u32>,
    next_size: Option<[f32; 2]>,
    rebake: Rebake,
}

impl Panel {
    pub(crate) fn begin_frame(&mut self, dockspace: u32, feedback: Feedback) {
        self.commands.clear();
        self.scope.clear();
        self.seen.clear();
        self.next_dock = None;
        self.next_size = None;
        self.rebake = Rebake::default();
        self.dockspace = dockspace;
        self.feedback = feedback;
        self.active = true;
    }

    pub(crate) fn end_frame(&mut self) -> (Vec<Command>, Rebake) {
        self.active = false;
        self.feedback = Feedback::default();
        // A profile that returns early leaves its windows and nodes open. Close them here so a
        // missing imgui_end() costs the profile nothing more than the frame it drew.
        for (scope, _) in self.scope.drain(..).rev() {
            self.commands.push(match scope {
                Scope::Window => Command::End,
                Scope::Tree => Command::TreeEnd,
            });
        }
        self.seen.clear();

        (std::mem::take(&mut self.commands), std::mem::take(&mut self.rebake))
    }

    /// Widget identity is the path of enclosing labels, so the same label under two nodes stays two
    /// widgets and a repeated one under the same node stays stable across frames.
    fn key(&mut self, label: &str) -> String {
        let mut key = String::new();
        for (_, scope) in &self.scope {
            key.push_str(scope);
            key.push('/');
        }
        key.push_str(label);

        let count = self.seen.entry(key.clone()).or_default();
        *count += 1;
        if *count > 1 {
            key.push('#');
            key.push_str(&count.to_string());
        }

        key
    }

    fn open(&self, key: &str) -> bool { !self.feedback.closed.contains(key) }

    fn clicked(&self, key: &str) -> bool { self.feedback.clicks.contains(key) }

    fn value(&self, key: &str) -> Option<&Value> { self.feedback.values.get(key) }
}

fn clamped(text: &str, limit: usize) -> String { text.chars().take(limit).collect() }

impl Evaluator<'_> {
    pub(crate) fn imgui(
        &mut self, intrinsic: Intrinsic, name: &str, args: Vec<(Option<Identifier>, GenericValue)>,
    ) -> Result<GenericValue> {
        if !self.runtime.ui.active {
            return Err(self.fault(FaultKind::Blocked(format!("{name} outside demir_ui"))));
        }

        let arg = |n: usize| args.get(n).map(|(_, value)| value.clone()).unwrap_or_default();
        let number = |n: usize| arg(n).num().filter(|value: &f32| value.is_finite()).unwrap_or_default();
        let label = |n: usize| clamped(&arg(n).display(), MAX_LABEL);

        match intrinsic {
            Intrinsic::DemirRebake => {
                let mask = |n: usize| {
                    let value = number(n);
                    if value >= 0.0 { value as u32 } else { 0 }
                };
                self.runtime.ui.rebake.request(mask(0), mask(1));

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiDockspace => Ok(GenericValue::Num(DOCKSPACE_HANDLE)),
            Intrinsic::ImguiSetNextWindowDock => {
                self.runtime.ui.next_dock = (number(0) == DOCKSPACE_HANDLE).then_some(self.runtime.ui.dockspace);

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiSetNextWindowSize => {
                self.runtime.ui.next_size = Some([number(0).max(0.0), number(1).max(0.0)]);

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiBegin => {
                let label = label(0);
                if self.runtime.ui.scope.len() >= MAX_DEPTH {
                    return Err(self.fault(FaultKind::InvalidOperation(format!(
                        "demir_ui nested more than {MAX_DEPTH} windows and nodes"
                    ))));
                }

                let key = self.runtime.ui.key(&label);
                let dock = self.runtime.ui.next_dock.take();
                let size = self.runtime.ui.next_size.take();
                self.emit(Command::Begin {
                    key: key.clone(),
                    label,
                    dock,
                    size,
                })?;
                let open = self.runtime.ui.open(&key);
                self.runtime.ui.scope.push((Scope::Window, key));

                Ok(open.into())
            },
            Intrinsic::ImguiEnd => {
                self.emit(Command::End)?;
                self.close(Scope::Window, name)?;

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiText | Intrinsic::ImguiTextColored => {
                let colored = intrinsic == Intrinsic::ImguiTextColored;
                let color = colored.then(|| parse_color(arg(0).text()));
                let text = clamped(&arg(usize::from(colored)).display(), MAX_TEXT);
                self.emit(Command::Text { text, color })?;

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiButton => {
                let label = label(0);
                let key = self.runtime.ui.key(&label);
                self.emit(Command::Button {
                    key: key.clone(),
                    label,
                })?;

                Ok(self.runtime.ui.clicked(&key).into())
            },
            Intrinsic::ImguiCheckbox => {
                let label = label(0);
                let key = self.runtime.ui.key(&label);
                let value = match self.runtime.ui.value(&key) {
                    Some(Value::Bool(edited)) => *edited,
                    _ => arg(1).truthy(),
                };
                self.emit(Command::Checkbox { key, label, value })?;

                Ok(value.into())
            },
            Intrinsic::ImguiRadio => {
                let label = label(0);
                let key = self.runtime.ui.key(&label);
                self.emit(Command::Radio {
                    key: key.clone(),
                    label,
                    active: arg(1).truthy(),
                })?;

                Ok(self.runtime.ui.clicked(&key).into())
            },
            Intrinsic::ImguiSlider | Intrinsic::ImguiDrag => {
                let label = label(0);
                let drag = intrinsic == Intrinsic::ImguiDrag;
                let range = if drag {
                    [number(3), number(4)]
                } else {
                    [number(2), number(3)]
                };
                let key = self.runtime.ui.key(&label);
                let value = match self.runtime.ui.value(&key) {
                    Some(Value::Num(edited)) => *edited,
                    _ => number(1),
                };
                self.emit(if drag {
                    Command::Drag {
                        key,
                        label,
                        value,
                        speed: number(2).max(0.0),
                        range,
                    }
                } else {
                    Command::Slider {
                        key,
                        label,
                        value,
                        range,
                    }
                })?;

                Ok(value.into())
            },
            Intrinsic::ImguiInputText => {
                let label = label(0);
                let key = self.runtime.ui.key(&label);
                let value = match self.runtime.ui.value(&key) {
                    Some(Value::Text(edited)) => clamped(edited, MAX_TEXT),
                    _ => clamped(&arg(1).display(), MAX_TEXT),
                };
                self.emit(Command::InputText {
                    key,
                    label,
                    value: value.clone(),
                })?;

                self.text(value)
            },
            Intrinsic::ImguiSeparator => {
                let label = label(0);
                self.emit(Command::Separator {
                    label: (!label.is_empty()).then_some(label),
                })?;

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiSameLine => {
                self.emit(Command::SameLine)?;

                Ok(GenericValue::Null)
            },
            Intrinsic::ImguiTree | Intrinsic::ImguiCollapsingHeader => {
                let label = label(0);
                let tree = intrinsic == Intrinsic::ImguiTree;
                if tree && self.runtime.ui.scope.len() >= MAX_DEPTH {
                    return Err(self.fault(FaultKind::InvalidOperation(format!(
                        "demir_ui nested more than {MAX_DEPTH} windows and nodes"
                    ))));
                }

                let key = self.runtime.ui.key(&label);
                self.emit(if tree {
                    Command::Tree {
                        key: key.clone(),
                        label,
                    }
                } else {
                    Command::CollapsingHeader {
                        key: key.clone(),
                        label,
                    }
                })?;

                let open = self.runtime.ui.open(&key);
                if tree {
                    // A profile pairs imgui_tree_end() with an open node only, so close a collapsed one
                    // here and keep the stream balanced for whoever replays it.
                    match open {
                        true => self.runtime.ui.scope.push((Scope::Tree, key)),
                        false => self.emit(Command::TreeEnd)?,
                    }
                }

                Ok(open.into())
            },
            Intrinsic::ImguiTreeEnd => {
                self.emit(Command::TreeEnd)?;
                self.close(Scope::Tree, name)?;

                Ok(GenericValue::Null)
            },
            _ => Err(self.fault(FaultKind::Blocked(name.into()))),
        }
    }

    fn emit(&mut self, command: Command) -> Result<()> {
        if self.runtime.ui.commands.len() >= MAX_COMMANDS {
            return Err(self.fault(FaultKind::InvalidOperation(format!(
                "demir_ui emitted more than {MAX_COMMANDS} commands"
            ))));
        }

        if !matches!(command, Command::Begin { .. }) && self.runtime.ui.scope.is_empty() {
            return Err(self.fault(FaultKind::InvalidOperation(
                "demir_ui drew outside of imgui_begin".into(),
            )));
        }

        self.charge(1)?;
        self.runtime.ui.commands.push(command);

        Ok(())
    }

    fn close(&mut self, scope: Scope, name: &str) -> Result<()> {
        if self.runtime.ui.scope.last().map(|(kind, _)| *kind) != Some(scope) {
            return Err(self.fault(FaultKind::InvalidOperation(format!("{name} closed nothing"))));
        }

        self.runtime.ui.scope.pop();

        Ok(())
    }
}
