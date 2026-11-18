pub(crate) mod fill;
pub(crate) mod panel;
pub(crate) mod place;
pub(crate) mod selection;
pub(crate) mod session;
pub(crate) mod transform;

use base::{coord::Coord, path::TreePath};

use objtree::ObjectTree;
use prefab::{Prefab, PrefabInstanceId};
use ui::icons::materialdesignicons::{
    ICON_ARROW_ALL, ICON_COMMENT_TEXT_OUTLINE, ICON_ERASER, ICON_EYEDROPPER, ICON_FIND_REPLACE, ICON_FORMAT_COLOR_FILL,
    ICON_PENCIL, ICON_SELECT_DRAG, ICON_TEXTURE, ICON_VECTOR_POLYLINE,
};

use crate::{
    document::{MapDocument, command::Edit, selection::Selection},
    environment::world::World,
};

use self::{
    fill::fill,
    place::{delete, place},
};

pub const MAX_FILL_TILES: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    Place,
    #[default]
    Pick,
    Node,
    Select,
    Delete,
    Replace,
    Fill,
    RectFill,
    Comment,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionTransform {
    RotateClockwise,
    RotateCounterClockwise,
    MirrorHorizontal,
    MirrorVertical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionRotation {
    #[default]
    Original,
    Clockwise,
    Half,
    CounterClockwise,
}

impl SelectionRotation {
    pub const fn clockwise(self) -> Self {
        match self {
            Self::Original => Self::Clockwise,
            Self::Clockwise => Self::Half,
            Self::Half => Self::CounterClockwise,
            Self::CounterClockwise => Self::Original,
        }
    }

    pub const fn counter_clockwise(self) -> Self {
        match self {
            Self::Original => Self::CounterClockwise,
            Self::Clockwise => Self::Original,
            Self::Half => Self::Clockwise,
            Self::CounterClockwise => Self::Half,
        }
    }

    const fn transforms(self) -> &'static [SelectionTransform] {
        match self {
            Self::Original => &[],
            Self::Clockwise => &[SelectionTransform::RotateClockwise],
            Self::Half => &[SelectionTransform::RotateClockwise, SelectionTransform::RotateClockwise],
            Self::CounterClockwise => &[SelectionTransform::RotateCounterClockwise],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionPlacement {
    Move,
    Copy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillMode {
    #[default]
    Wall,
    EntireArea,
    Custom,
}

impl FillMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
            Self::EntireArea => "Entire Area",
            Self::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RegionSelectionMode {
    #[default]
    Full,
    Hollow {
        line_width: u32,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionMask {
    pub bounds: Selection,
    pub mode: RegionSelectionMode,
}

impl SelectionMask {
    pub fn includes(self, coord: Coord) -> bool {
        self.mode.includes(self.bounds, coord)
    }
}

impl RegionSelectionMode {
    /// all the tiles of `selection` this mode actually edits, hollow mode will
    /// only edit tiles that are inside the ring
    pub fn tiles(self, selection: Selection) -> impl Iterator<Item = Coord> {
        selection.iter().filter(move |coord| self.includes(selection, *coord))
    }

    pub fn includes(self, selection: Selection, coord: Coord) -> bool {
        if !selection.contains(coord) {
            return false;
        }

        match self {
            Self::Full => true,
            Self::Hollow { line_width } => {
                let line_width = line_width.max(1);

                coord.x - selection.min.x < line_width
                    || selection.max.x - coord.x < line_width
                    || coord.y - selection.min.y < line_width
                    || selection.max.y - coord.y < line_width
            },
        }
    }
}

pub struct ToolContext<'a> {
    pub document: &'a mut MapDocument,
    pub tree: &'a ObjectTree,
    pub world: &'a World,
    pub prefab: Option<&'a Prefab>,
    pub target: Option<PrefabInstanceId>,
    pub coord: Coord,
    pub anchor: Option<Coord>,
    pub fill_mode: FillMode,
    pub custom_fill_boundaries: &'a [TreePath],
}

pub struct ToolEdit {
    pub edit: Edit,
    pub picked: Option<PrefabInstanceId>,
    pub affected: Vec<PrefabInstanceId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillError {
    TooLarge { limit: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlacementKind {
    Atom,
    Turf,
    Area,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Place => "Place",
            Tool::Pick => "Pick",
            Tool::Node => "Node",
            Tool::Select => "Select",
            Tool::Delete => "Delete",
            Tool::Replace => "Replace",
            Tool::Fill => "Fill",
            Tool::RectFill => "Rectangle Fill",
            Tool::Comment => "Comment",
            Tool::Move => "Move",
        }
    }

    pub fn icon(self) -> char {
        match self {
            Tool::Place => ICON_PENCIL,
            Tool::Pick => ICON_EYEDROPPER,
            Tool::Node => ICON_VECTOR_POLYLINE,
            Tool::Select => ICON_SELECT_DRAG,
            Tool::Delete => ICON_ERASER,
            Tool::Replace => ICON_FIND_REPLACE,
            Tool::Fill => ICON_FORMAT_COLOR_FILL,
            Tool::RectFill => ICON_TEXTURE,
            Tool::Comment => ICON_COMMENT_TEXT_OUTLINE,
            Tool::Move => ICON_ARROW_ALL,
        }
    }

    pub fn build_edit(self, context: &mut ToolContext<'_>) -> Option<ToolEdit> {
        match self {
            Self::Place => place(context),
            Self::Delete => delete(context),
            Self::Pick | Self::Node | Self::Select | Self::Replace | Self::RectFill | Self::Comment | Self::Move => {
                None
            },
            Self::Fill => fill(context, Some(MAX_FILL_TILES), None).ok().flatten(),
        }
    }

    pub fn build_fill_edit(
        self, context: &mut ToolContext<'_>, max_tiles: Option<usize>,
    ) -> Result<Option<ToolEdit>, FillError> {
        self.build_fill_edit_with_mask(context, max_tiles, None)
    }

    pub fn build_fill_edit_with_mask(
        self, context: &mut ToolContext<'_>, max_tiles: Option<usize>, mask: Option<SelectionMask>,
    ) -> Result<Option<ToolEdit>, FillError> {
        if self == Self::Fill {
            fill(context, max_tiles, mask)
        } else {
            Ok(self.build_edit(context))
        }
    }
}

impl From<Tool> for net::Tool {
    fn from(tool: Tool) -> Self {
        match tool {
            Tool::Place => Self::Place,
            Tool::Pick => Self::Pick,
            Tool::Node => Self::Node,
            Tool::Select => Self::Select,
            Tool::Delete => Self::Delete,
            Tool::Replace => Self::Replace,
            Tool::Fill | Tool::RectFill => Self::Fill,
            Tool::Comment => Self::Comment,
            Tool::Move => Self::Move,
        }
    }
}

impl From<net::Tool> for Tool {
    fn from(tool: net::Tool) -> Self {
        match tool {
            net::Tool::Place => Self::Place,
            net::Tool::Pick => Self::Pick,
            net::Tool::Node => Self::Node,
            net::Tool::Select => Self::Select,
            net::Tool::Delete => Self::Delete,
            net::Tool::Replace => Self::Replace,
            net::Tool::Fill => Self::Fill,
            net::Tool::Comment => Self::Comment,
            net::Tool::Move => Self::Move,
        }
    }
}

impl From<RegionSelectionMode> for net::SelectionMode {
    fn from(mode: RegionSelectionMode) -> Self {
        match mode {
            RegionSelectionMode::Full => Self::Full,
            RegionSelectionMode::Hollow { line_width } => Self::Hollow { line_width },
        }
    }
}

impl From<net::SelectionMode> for RegionSelectionMode {
    fn from(mode: net::SelectionMode) -> Self {
        match mode {
            net::SelectionMode::Full => Self::Full,
            net::SelectionMode::Hollow { line_width } => Self::Hollow { line_width },
        }
    }
}

#[cfg(test)]
mod tests;
