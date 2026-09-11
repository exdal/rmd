use ash::vk;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    Loader(String),
    Vulkan(vk::Result),
    NoSuitableDevice,
    NoGraphicsQueue,
    UnsupportedWindow,
    TooManyTextures {
        requested: u32,
        limit: u32,
    },
    TextureCapacityExceeded {
        requested: u32,
        capacity: u32,
    },
    TextureUploadTooLarge,
    TextureLoad {
        path: String,
        message: String,
    },
    TextureSourceChanged {
        path: String,
        expected_width: u32,
        expected_height: u32,
        actual_width: u32,
        actual_height: u32,
    },
    TextureSheetTooLarge {
        path: String,
        width: u32,
        height: u32,
        limit: u32,
    },
    SpritePackingOutOfRange {
        sprite: usize,
        field: &'static str,
    },
    SpriteUploadTooLarge,
    TextureIdExhausted,
    OutOfDate,
    RawDrawCallback,
    TextureRowPitchTooSmall,
    TextureUpdateMismatch,
    UnknownTexture(u64),
    ImGui(String),
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loader(e) => write!(f, "could not load Vulkan: {e}"),
            Self::Vulkan(e) => write!(f, "vulkan error: {e}"),
            Self::NoSuitableDevice => write!(
                f,
                "no Vulkan 1.3 device with a graphics and compute queue, a swapchain, and bindless sampled-image \
                 support"
            ),
            Self::NoGraphicsQueue => write!(f, "the selected device has no graphics and compute queue"),
            Self::UnsupportedWindow => write!(f, "unsupported window system"),
            Self::TooManyTextures { requested, limit } => {
                write!(
                    f,
                    "the scene needs {requested} bindless textures, but the device supports {limit}"
                )
            },
            Self::TextureCapacityExceeded { requested, capacity } => write!(
                f,
                "the renderer was created for {capacity} DMI sheets, but this catalog contains {requested}"
            ),
            Self::SpriteUploadTooLarge => write!(f, "sprite data is too large to upload"),
            Self::TextureUploadTooLarge => write!(f, "sprite texture data is too large to upload"),
            Self::TextureLoad { path, message } => write!(f, "could not decode texture '{path}': {message}"),
            Self::TextureSourceChanged {
                path,
                expected_width,
                expected_height,
                actual_width,
                actual_height,
            } => write!(
                f,
                "texture '{path}' changed size after it was catalogued: expected {expected_width}x{expected_height}, \
                 got {actual_width}x{actual_height}"
            ),
            Self::TextureSheetTooLarge {
                path,
                width,
                height,
                limit,
            } => write!(
                f,
                "texture '{path}' is {width}x{height}, but this device supports at most {limit}x{limit}"
            ),
            Self::SpritePackingOutOfRange { sprite, field } => {
                write!(
                    f,
                    "sprite {sprite} has a {field} value outside the packed GPU representation"
                )
            },
            Self::TextureIdExhausted => write!(f, "the interface exhausted the renderer texture id space"),
            Self::OutOfDate => write!(f, "the swapchain is out of date"),
            Self::RawDrawCallback => write!(
                f,
                "the interface asked for a raw draw callback, which this backend has not"
            ),
            Self::TextureRowPitchTooSmall => write!(f, "an interface texture upload is narrower than one packed row"),
            Self::TextureUpdateMismatch => write!(f, "an interface texture patch does not fit the image it names"),
            Self::UnknownTexture(id) => write!(f, "the interface painted with texture {id}, which was never uploaded"),
            Self::ImGui(e) => write!(f, "imgui error: {e}"),
        }
    }
}

impl std::error::Error for GpuError {}

impl From<vk::Result> for GpuError {
    fn from(result: vk::Result) -> Self {
        match result {
            vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR => Self::OutOfDate,
            other => Self::Vulkan(other),
        }
    }
}
