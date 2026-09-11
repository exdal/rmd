pub mod error;
pub mod metadata;

use std::{
    fs::File,
    io::{BufRead, BufReader, Seek},
    path::{Path, PathBuf},
};

use png::{ColorType, Decoder, Limits, OutputInfo, Reader, Transformations};

use crate::{error::IconError, metadata::Metadata};

const DECODE_LIMIT: usize = 1 << 28;

pub struct IconFile {
    pub path: PathBuf,
    pub metadata: Metadata,
    pub sheet_width: u32,
    pub sheet_height: u32,
    /// RGBA8 row major
    pub pixels: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IconInfo {
    pub path: PathBuf,
    pub metadata: Metadata,
    pub sheet_width: u32,
    pub sheet_height: u32,
}

/// `zTXt "Description"`
fn read_description<R: BufRead + Seek>(reader: R) -> Result<(Metadata, Reader<R>), IconError> {
    let mut decoder = Decoder::new(reader);
    decoder.set_limits(Limits { bytes: DECODE_LIMIT });
    decoder.set_transformations(Transformations::EXPAND | Transformations::ALPHA | Transformations::STRIP_16);

    let reader = decoder.read_info()?;
    let info = reader.info();

    let description = info
        .compressed_latin1_text
        .iter()
        .find(|chunk| chunk.keyword == "Description")
        .map(|chunk| chunk.get_text())
        .or_else(|| {
            info.uncompressed_latin1_text
                .iter()
                .find(|chunk| chunk.keyword == "Description")
                .map(|chunk| Ok(chunk.text.clone()))
        })
        .or_else(|| {
            info.utf8_text
                .iter()
                .find(|chunk| chunk.keyword == "Description")
                .map(|chunk| chunk.get_text())
        });

    let Some(description) = description else {
        return Err(IconError::MissingMetadata);
    };

    let metadata = Metadata::parse_description(&description?)?;

    Ok((metadata, reader))
}

fn read_icon<R: BufRead + Seek>(reader: R) -> Result<(Metadata, u32, u32, Vec<u8>), IconError> {
    let (metadata, mut reader) = read_description(reader)?;

    let Some(size) = reader.output_buffer_size() else {
        return Err(IconError::Decode(String::from("sheet too large to decode")));
    };

    let mut buffer = vec![0; size];
    let info = reader.next_frame(&mut buffer)?;
    buffer.truncate(info.buffer_size());
    let pixels = to_rgba8(&info, buffer)?;

    Ok((metadata, info.width, info.height, pixels))
}

fn to_rgba8(info: &OutputInfo, buffer: Vec<u8>) -> Result<Vec<u8>, IconError> {
    if info.color_type == ColorType::Rgba {
        return Ok(buffer);
    }

    let pixels = (info.width as usize).saturating_mul(info.height as usize);
    let mut rgba = Vec::with_capacity(pixels.saturating_mul(4));

    match info.color_type {
        ColorType::Rgb => {
            for pixel in buffer.chunks_exact(3) {
                rgba.extend_from_slice(pixel);
                rgba.push(255);
            }
        },
        ColorType::GrayscaleAlpha => {
            for pixel in buffer.chunks_exact(2) {
                if let [gray, alpha] = *pixel {
                    rgba.extend_from_slice(&[gray, gray, gray, alpha]);
                }
            }
        },
        ColorType::Grayscale => {
            for gray in buffer.iter().copied() {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        },
        ColorType::Rgba => {},
        ColorType::Indexed => return Err(IconError::Decode(String::from("palette survived expansion"))),
    }

    Ok(rgba)
}

impl IconFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, IconError> {
        let path = path.as_ref();
        let (metadata, sheet_width, sheet_height, pixels) = read_icon(BufReader::new(File::open(path)?))?;

        Ok(Self {
            path: path.to_path_buf(),
            metadata,
            sheet_width,
            sheet_height,
            pixels,
        })
    }

    pub fn load_metadata(path: impl AsRef<Path>) -> Result<Metadata, IconError> { Ok(Self::load_info(path)?.metadata) }

    pub fn load_info(path: impl AsRef<Path>) -> Result<IconInfo, IconError> {
        let path = path.as_ref();
        let (metadata, reader) = read_description(BufReader::new(File::open(path)?))?;
        let info = reader.info();

        Ok(IconInfo {
            path: path.to_path_buf(),
            metadata,
            sheet_width: info.width,
            sheet_height: info.height,
        })
    }

    pub fn cell_count(&self) -> usize { cell_count(&self.metadata, self.sheet_width, self.sheet_height) }
}

impl IconInfo {
    pub fn cell_count(&self) -> usize { cell_count(&self.metadata, self.sheet_width, self.sheet_height) }
}

fn cell_count(metadata: &Metadata, sheet_width: u32, sheet_height: u32) -> usize {
    if metadata.width == 0 || metadata.height == 0 {
        return 0;
    }

    let columns = (sheet_width / metadata.width) as usize;
    let rows = (sheet_height / metadata.height) as usize;

    columns * rows
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use png::{BitDepth, ColorType, Encoder};

    use crate::{IconError, metadata::Dir, read_description, read_icon};

    const DESCRIPTION: &str = "# BEGIN DMI\nversion = 4.0\n\twidth = 2\n\theight = 2\nstate = \"chair\"\n\tdirs = \
                               1\n\tframes = 1\n# END DMI\n";

    fn encode(color: ColorType, depth: BitDepth, data: &[u8], description: Option<&str>) -> Vec<u8> {
        let mut out = Vec::new();
        let mut encoder = Encoder::new(Cursor::new(&mut out), 2, 2);
        encoder.set_color(color);
        encoder.set_depth(depth);

        if color == ColorType::Indexed {
            encoder.set_palette(vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
            encoder.set_trns(vec![0, 255, 255, 255]);
        }

        if let Some(description) = description {
            encoder
                .add_ztxt_chunk(String::from("Description"), String::from(description))
                .expect("ztxt");
        }

        let mut writer = encoder.write_header().expect("header");
        writer.write_image_data(data).expect("image data");
        writer.finish().expect("finish");

        out
    }

    #[test]
    fn decodes_rgba8() {
        let data = [255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 255, 255, 255, 255, 0];
        let bytes = encode(ColorType::Rgba, BitDepth::Eight, &data, Some(DESCRIPTION));

        let (metadata, width, height, pixels) = read_icon(Cursor::new(bytes)).expect("decode");
        assert_eq!((width, height), (2, 2));
        assert_eq!(pixels, data);
        assert_eq!(metadata.width, 2);
        assert_eq!(metadata.states.len(), 1);
        assert_eq!(metadata.states[0].name, "chair");
        assert_eq!(metadata.states[0].sprite_index(Dir::South, 0), 0);
    }

    /// `EXPAND | ALPHA`
    #[test]
    fn expands_palette_with_trns() {
        let bytes = encode(ColorType::Indexed, BitDepth::Eight, &[0, 1, 2, 3], Some(DESCRIPTION));

        let (_, _, _, pixels) = read_icon(Cursor::new(bytes)).expect("decode");
        assert_eq!(
            pixels,
            [255, 0, 0, 0, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn widens_grayscale_alpha() {
        let bytes = encode(
            ColorType::GrayscaleAlpha,
            BitDepth::Eight,
            &[10, 255, 20, 0, 30, 128, 40, 255],
            Some(DESCRIPTION),
        );

        let (_, _, _, pixels) = read_icon(Cursor::new(bytes)).expect("decode");
        assert_eq!(
            pixels,
            [10, 10, 10, 255, 20, 20, 20, 0, 30, 30, 30, 128, 40, 40, 40, 255]
        );
    }

    #[test]
    fn widens_grayscale() {
        let bytes = encode(
            ColorType::Grayscale,
            BitDepth::Eight,
            &[10, 20, 30, 40],
            Some(DESCRIPTION),
        );

        let (_, _, _, pixels) = read_icon(Cursor::new(bytes)).expect("decode");
        assert_eq!(
            pixels,
            [10, 10, 10, 255, 20, 20, 20, 255, 30, 30, 30, 255, 40, 40, 40, 255]
        );
    }

    #[test]
    fn plain_png_is_not_a_dmi() {
        let bytes = encode(ColorType::Rgba, BitDepth::Eight, &[0; 16], None);

        assert!(matches!(read_icon(Cursor::new(bytes)), Err(IconError::MissingMetadata)));
    }

    #[test]
    fn garbage_does_not_panic() {
        assert!(matches!(
            read_icon(Cursor::new(b"not a png at all".to_vec())),
            Err(IconError::Decode(_))
        ));

        let mut truncated = encode(ColorType::Rgba, BitDepth::Eight, &[0; 16], Some(DESCRIPTION));
        truncated.truncate(truncated.len() / 2);
        assert!(read_icon(Cursor::new(truncated)).is_err());
    }

    #[test]
    fn metadata_only_matches_full_read() {
        let bytes = encode(ColorType::Rgba, BitDepth::Eight, &[0; 16], Some(DESCRIPTION));

        let (full, ..) = read_icon(Cursor::new(bytes.clone())).expect("decode");
        let (metadata, reader) = read_description(Cursor::new(bytes)).expect("metadata");
        assert_eq!(full, metadata);
        assert_eq!((reader.info().width, reader.info().height), (2, 2));
    }
}
