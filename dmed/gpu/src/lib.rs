pub mod device;
pub mod editor;
pub mod error;
pub mod imgui;
pub mod renderer;
mod resources;
pub mod sprite;
pub mod upload;

pub use crate::{
    device::Device,
    editor::EditorRenderer,
    error::GpuError,
    imgui::ImGuiPass,
    renderer::VirRenderer,
    sprite::SpritePass,
};

pub(crate) fn read_spirv(bytes: &[u8]) -> Result<Vec<u32>, GpuError> {
    let mut cursor = std::io::Cursor::new(bytes);

    ash::util::read_spv(&mut cursor).map_err(|e| GpuError::Loader(e.to_string()))
}
