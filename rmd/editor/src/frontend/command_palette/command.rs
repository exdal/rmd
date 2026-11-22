use crate::{
    frame::session::TypeLayer,
    frontend::{LAYER_KEYS, SCREENSHOT_KEYS, ScreenshotArea, TOOL_KEYS},
    session::Session,
    settings::{Panel, keybind::KeybindAction},
    tool::Tool,
    viewport::panel::edit::{EDIT_KEYS, EditCommand},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Command {
    Tool(Tool),
    Layer(TypeLayer),
    Screenshot { area: ScreenshotArea, copy: bool },
    Edit(EditCommand),
    ShowPanel(Panel),
    Save,
    SaveAll,
    CloseMap,
    CloseAll,
    Undo,
    Redo,
    Find,
    GoTo,
    ResizeMap,
    NewMap,
    OpenMap,
    OpenCodebase,
    ShowAllLayers,
    ToggleAreas,
    ToggleAreaOutlines,
    ToggleLighting,
    ToggleTileGrid,
    TogglePixelGrid,
    ToggleMirrorCamera,
    LevelUp,
    LevelDown,
    Refit,
    ToggleDmUi,
    ResetLayout,
    Settings,
    Welcome,
    About,
}

impl Command {
    pub(crate) fn all() -> Vec<Self> {
        let tools = TOOL_KEYS.into_iter().map(|(_, tool)| Self::Tool(tool));
        let files = [
            Self::NewMap,
            Self::OpenMap,
            Self::OpenCodebase,
            Self::Save,
            Self::SaveAll,
            Self::CloseMap,
            Self::CloseAll,
        ];
        let screenshots = SCREENSHOT_KEYS
            .into_iter()
            .map(|(_, area, copy)| Self::Screenshot { area, copy });
        let history = [Self::Undo, Self::Redo];
        let edits = EDIT_KEYS.into_iter().map(|(_, command)| Self::Edit(command));
        let navigation = [Self::Find, Self::GoTo, Self::ResizeMap];
        let layers = LAYER_KEYS.into_iter().map(|(_, layer)| Self::Layer(layer));
        let view = [
            Self::ShowAllLayers,
            Self::ToggleAreas,
            Self::ToggleAreaOutlines,
            Self::ToggleLighting,
            Self::ToggleTileGrid,
            Self::TogglePixelGrid,
            Self::ToggleMirrorCamera,
            Self::LevelUp,
            Self::LevelDown,
            Self::Refit,
        ];
        let panels = Panel::ALL.into_iter().map(Self::ShowPanel);
        let application = [
            Self::ToggleDmUi,
            Self::ResetLayout,
            Self::Settings,
            Self::Welcome,
            Self::About,
        ];

        tools
            .chain(files)
            .chain(screenshots)
            .chain(history)
            .chain(edits)
            .chain(navigation)
            .chain(layers)
            .chain(view)
            .chain(panels)
            .chain(application)
            .collect::<Vec<_>>()
    }

    pub(crate) fn keybind(self) -> Option<KeybindAction> {
        match self {
            Self::Tool(tool) => TOOL_KEYS
                .into_iter()
                .find(|&(_, candidate)| candidate == tool)
                .map(|(action, _)| action),
            Self::Layer(layer) => LAYER_KEYS
                .into_iter()
                .find(|&(_, candidate)| candidate == layer)
                .map(|(action, _)| action),
            Self::Screenshot { area, copy } => SCREENSHOT_KEYS
                .into_iter()
                .find(|&(_, candidate_area, candidate_copy)| candidate_area == area && candidate_copy == copy)
                .map(|(action, ..)| action),
            Self::Edit(command) => EDIT_KEYS
                .into_iter()
                .find(|&(_, candidate)| candidate == command)
                .map(|(action, _)| action),
            Self::Save => Some(KeybindAction::Save),
            Self::SaveAll => Some(KeybindAction::SaveAll),
            Self::CloseMap => Some(KeybindAction::CloseMap),
            Self::Undo => Some(KeybindAction::Undo),
            Self::Redo => Some(KeybindAction::Redo),
            Self::Find => Some(KeybindAction::Find),
            Self::GoTo => Some(KeybindAction::GoTo),
            Self::ShowAllLayers => Some(KeybindAction::ShowAllLayers),
            Self::ToggleAreas => Some(KeybindAction::ShowAreas),
            Self::ToggleAreaOutlines => Some(KeybindAction::ShowAreaOutlines),
            Self::ToggleLighting => Some(KeybindAction::ShowLighting),
            Self::ToggleTileGrid => Some(KeybindAction::ShowTileGrid),
            Self::TogglePixelGrid => Some(KeybindAction::ShowPixelGrid),
            Self::LevelUp => Some(KeybindAction::LevelUp),
            Self::LevelDown => Some(KeybindAction::LevelDown),
            Self::Refit => Some(KeybindAction::Refit),
            Self::ShowPanel(_)
            | Self::CloseAll
            | Self::ResizeMap
            | Self::NewMap
            | Self::OpenMap
            | Self::OpenCodebase
            | Self::ToggleMirrorCamera
            | Self::ToggleDmUi
            | Self::ResetLayout
            | Self::Settings
            | Self::Welcome
            | Self::About => None,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::ShowPanel(panel) => match panel {
                Panel::ObjectTree => "Show object tree panel",
                Panel::Git => "Show Git panel",
                Panel::Inspector => "Show inspector panel",
                Panel::Search => "Show search panel",
                Panel::MapIssues => "Show map issues panel",
                Panel::Runtimes => "Show runtimes panel",
                Panel::Stash => "Show stash panel",
            },
            Self::CloseAll => "Close all",
            Self::ResizeMap => "Resize map",
            Self::NewMap => "New map",
            Self::OpenMap => "Open map",
            Self::OpenCodebase => "Open codebase",
            Self::ToggleMirrorCamera => "Mirror camera",
            Self::ToggleDmUi => "Toggle DM UI",
            Self::ResetLayout => "Reset layout",
            Self::Settings => "Settings",
            Self::Welcome => "Welcome",
            Self::About => "About",
            _ => self.keybind().map_or("", KeybindAction::label),
        }
    }

    pub(crate) fn is_enabled(self, session: &Session) -> bool {
        let has_map = session.map().is_some();
        let is_stash = session.is_stash_active();
        let is_selected = session.selection().is_some();

        match self {
            Self::Tool(Tool::Node) => has_map && session.node_tool_available(),
            Self::Tool(Tool::Comment) => has_map && session.comment_tool_available(),
            Self::Tool(_) => has_map,
            Self::Layer(_) => session.tree().is_some(),
            Self::Screenshot { area, .. } => has_map && (area == ScreenshotArea::Map || is_selected),
            Self::Edit(command) => command.is_enabled(session),
            Self::Save | Self::CloseMap | Self::ResizeMap => has_map && !is_stash,
            Self::SaveAll => session.state.documents().iter().any(|document| document.is_dirty()),
            Self::CloseAll => !session.state.is_empty(),
            Self::Undo => session.undo_label().is_some(),
            Self::Redo => session.redo_label().is_some(),
            Self::Find | Self::GoTo => has_map,
            Self::NewMap | Self::OpenMap => session.tree().is_some(),
            Self::OpenCodebase => session.tree().is_none(),
            Self::ShowAllLayers => session.hides_types(),
            Self::ShowPanel(_)
            | Self::ToggleAreas
            | Self::ToggleAreaOutlines
            | Self::ToggleLighting
            | Self::ToggleTileGrid
            | Self::TogglePixelGrid
            | Self::ToggleMirrorCamera
            | Self::LevelUp
            | Self::LevelDown
            | Self::Refit
            | Self::ToggleDmUi
            | Self::ResetLayout
            | Self::Settings
            | Self::Welcome
            | Self::About => true,
        }
    }
}
