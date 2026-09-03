use dmm::{Coord, Prefab};

use crate::{command::Edit, document::MapDocument};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tool {
    #[default]
    Place,
    Select,
    Delete,
    Pick,
    Fill,
}

pub struct ToolContext<'a> {
    pub document: &'a mut MapDocument,
    pub prefab: Option<&'a Prefab>,
    pub coord: Coord,
    pub anchor: Option<Coord>,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Tool::Place => "Place",
            Tool::Select => "Select",
            Tool::Delete => "Delete",
            Tool::Pick => "Pick",
            Tool::Fill => "Fill",
        }
    }

    pub fn build_edit(self, context: &ToolContext<'_>) -> Option<Edit> {
        let _ = context;

        // TODO: implement each tool

        todo!("tool edits")
    }
}
