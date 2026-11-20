use ash::vk;

const MAYBE: &str = "If you have a graphics card that supports Vulkan 1.2, create an issue on GitHub with your GPU (I might just add support for it)";
const USUALLY_THE_SOLUTION: &str = "Install or update the graphics driver";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GpuError {
    Loader(String),
    InvalidShader(String),
    InstanceTooOld {
        found: u32,
    },
    MissingInstanceExtension(String),
    Vulkan(vk::Result),
    NoSuitableDevice(Vec<RejectedDevice>),
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
    CaptureUnavailable,
    CaptureTooLarge,
}

impl std::fmt::Display for GpuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Loader(e) => write!(
                f,
                "the Vulkan runtime could not be loaded ({e}): {USUALLY_THE_SOLUTION}"
            ),
            Self::InvalidShader(e) => write!(
                f,
                "invalid shader bytecode: {e}. Not sure how you got this but please report this"
            ),
            Self::InstanceTooOld { found } => write!(
                f,
                "the Vulkan runtime is version {}, but Vulkan 1.3 is required: {USUALLY_THE_SOLUTION}.\n{MAYBE}",
                format_version(*found)
            ),
            Self::MissingInstanceExtension(name) => write!(
                f,
                "the Vulkan runtime lacks the {name} instance extension: {USUALLY_THE_SOLUTION}"
            ),
            Self::Vulkan(e) => write!(f, "vulkan error: {e}"),
            Self::NoSuitableDevice(rejected) => {
                if rejected.is_empty() {
                    return write!(f, "no Vulkan devices were found");
                }

                write!(f, "no GPU supports what this application needs:")?;
                for device in rejected {
                    write!(f, "\n- {device}")?;
                }

                Ok(())
            },
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
            Self::CaptureUnavailable => write!(f, "the map to capture has not been drawn yet"),
            Self::CaptureTooLarge => write!(f, "the map is too large to capture"),
        }
    }
}

impl std::error::Error for GpuError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shortfall {
    ApiVersion { found: u32 },
    ExtensionQueryFailed(vk::Result),
    NoSwapchain,
    MissingFeature(&'static str),
    MissingSubgroupOps,
    NoGraphicsComputeQueue,
    NoBindlessTextures,
}

impl std::fmt::Display for Shortfall {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ApiVersion { found } => write!(
                f,
                "supports Vulkan {}, but Vulkan 1.3 is required",
                format_version(*found)
            ),
            Self::ExtensionQueryFailed(e) => write!(f, "its extensions could not be listed ({e})"),
            Self::NoSwapchain => write!(f, "it cannot present to a window (no swapchain support)"),
            Self::MissingFeature(name) => write!(f, "it lacks the {name} feature"),
            Self::MissingSubgroupOps => write!(f, "it lacks basic and ballot subgroup operations in compute shaders"),
            Self::NoGraphicsComputeQueue => write!(f, "it has no queue that supports both graphics and compute"),
            Self::NoBindlessTextures => write!(f, "it does not support bindless texture arrays"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedDevice {
    pub name: String,
    pub shortfalls: Vec<Shortfall>,
}

impl std::fmt::Display for RejectedDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: ", self.name)?;
        for (index, shortfall) in self.shortfalls.iter().enumerate() {
            if index > 0 {
                write!(f, ", ")?;
            }

            write!(f, "{shortfall}")?;
        }

        Ok(())
    }
}

fn format_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        vk::api_version_major(version),
        vk::api_version_minor(version),
        vk::api_version_patch(version)
    )
}

impl From<vk::Result> for GpuError {
    fn from(result: vk::Result) -> Self {
        match result {
            vk::Result::ERROR_OUT_OF_DATE_KHR | vk::Result::SUBOPTIMAL_KHR => Self::OutOfDate,
            other => Self::Vulkan(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use ash::vk;

    use crate::error::{GpuError, RejectedDevice, Shortfall};

    #[test]
    fn no_devices_is_reported_plainly() {
        assert_eq!(
            GpuError::NoSuitableDevice(Vec::new()).to_string(),
            "no Vulkan devices were found"
        );
    }

    #[test]
    fn every_shortfall_of_every_device_is_listed() {
        let error = GpuError::NoSuitableDevice(vec![
            RejectedDevice {
                name: "Old GPU".into(),
                shortfalls: vec![Shortfall::ApiVersion {
                    found: vk::make_api_version(0, 1, 2, 198),
                }],
            },
            RejectedDevice {
                name: "Other GPU".into(),
                shortfalls: vec![Shortfall::NoSwapchain, Shortfall::MissingFeature("independentBlend")],
            },
        ]);

        assert_eq!(
            error.to_string(),
            "no GPU supports what this application needs:\n\
             - Old GPU: supports Vulkan 1.2.198, but Vulkan 1.3 is required\n\
             - Other GPU: it cannot present to a window (no swapchain support), it lacks the independentBlend feature"
        );
    }

    #[test]
    fn old_instance_version_is_formatted() {
        let error = GpuError::InstanceTooOld {
            found: vk::make_api_version(0, 1, 1, 7),
        };

        assert!(error.to_string().contains("version 1.1.7,"));
    }
}
