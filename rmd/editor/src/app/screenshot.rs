use std::{
    fs,
    path::{Path, PathBuf},
};

use dear_imgui_rs::ClipboardBackend;
use render::{CapturedImage, Renderer};

use crate::{
    document::{DocumentId, Selection},
    session::Session,
};

pub(super) enum ScreenshotTarget {
    File(PathBuf),
    Clipboard,
}

pub(super) struct PendingScreenshot {
    pub(super) target: ScreenshotTarget,
    pub(super) selection: Option<Selection>,
}

pub(super) fn take_screenshot(
    renderer: &mut Renderer, session: &Session, scene: &render::Frame<'_>, drawn: &[DocumentId],
    pending: PendingScreenshot, clipboard: &mut Option<arboard::Clipboard>,
) {
    let Some(document) = session.state.active() else {
        return;
    };
    let Some(view) = drawn.iter().position(|id| *id == document) else {
        log::error!("could not take a screenshot: the map is not open in a view");
        return;
    };
    let Some((origin, size)) = session.capture_region(document, pending.selection) else {
        log::error!("could not take a screenshot: the selection is outside the map");
        return;
    };
    let image = match renderer.capture(scene, view, origin, size) {
        Ok(image) => image,
        Err(e) => {
            log::error!("could not take a screenshot: {e}");
            return;
        },
    };

    match pending.target {
        ScreenshotTarget::File(path) => {
            std::thread::spawn(move || match write_png(&path, &image) {
                Ok(()) => log::info!("saved screenshot to {}", path.display()),
                Err(e) => log::error!("could not save screenshot to {}: {e}", path.display()),
            });
        },
        ScreenshotTarget::Clipboard => copy_image(clipboard, image),
    }
}

pub(super) struct SystemClipboard(pub(super) arboard::Clipboard);

impl ClipboardBackend for SystemClipboard {
    fn get(&mut self) -> Option<String> {
        self.0.get_text().ok()
    }

    fn set(&mut self, value: &str) {
        if let Err(e) = self.0.set_text(value) {
            log::error!("could not copy to the clipboard: {e}");
        }
    }
}

pub(super) fn copy_image(clipboard: &mut Option<arboard::Clipboard>, image: CapturedImage) {
    let clipboard = match clipboard {
        Some(clipboard) => clipboard,
        None => match arboard::Clipboard::new() {
            Ok(created) => clipboard.insert(created),
            Err(e) => {
                log::error!("could not open the clipboard: {e}");
                return;
            },
        },
    };
    let data = arboard::ImageData {
        width: image.width as usize,
        height: image.height as usize,
        bytes: image.rgba.into(),
    };

    match clipboard.set_image(data) {
        Ok(()) => log::info!("copied screenshot to the clipboard"),
        Err(e) => log::error!("could not copy the screenshot: {e}"),
    }
}

pub(super) fn write_png(path: &Path, image: &CapturedImage) -> Result<(), png::EncodingError> {
    let file = std::io::BufWriter::new(fs::File::create(path)?);
    let mut encoder = png::Encoder::new(file, image.width, image.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(&image.rgba)?;

    writer.finish()
}
