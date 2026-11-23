use std::{
    collections::HashMap,
    io::{Error, ErrorKind, Result},
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::is_valid_media_path_name;

mod capture;
pub use capture::V4l2Capture;

/// A local MJPEG color capture source encoded on demand for remote playback.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V4l2CameraConfig {
    pub id: String,
    pub description: Option<String>,
    pub device: PathBuf,
    pub width: u16,
    pub height: u16,
    pub frames_per_second: u8,
}

impl V4l2CameraConfig {
    pub(super) fn validate(&self) -> std::result::Result<(), String> {
        if !is_valid_media_path_name(&self.id) {
            return Err("V4L2 source ID must contain only letters, numbers, _ and -".into());
        }
        if !is_device_path(&self.device) {
            return Err(format!(
                "V4L2 source {} requires a device under /dev",
                self.id
            ));
        }
        if self.width == 0
            || self.height == 0
            || !self.width.is_multiple_of(2)
            || !self.height.is_multiple_of(2)
            || !(1..=60).contains(&self.frames_per_second)
        {
            return Err(format!(
                "V4L2 source {} requires positive even dimensions and 1 to 60 fps",
                self.id
            ));
        }
        Ok(())
    }

    pub fn supports_mjpeg(&self) -> Result<bool> {
        supports_capture(&self.device, &[*b"MJPG"])
    }
}

fn is_device_path(path: &Path) -> bool {
    path.starts_with("/dev")
        && path != Path::new("/dev")
        && !path.components().any(|part| part == Component::ParentDir)
}

/// The capture interfaces of one operator-configured USB depth camera.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DepthCamera {
    depth_device: PathBuf,
    color_device: Option<PathBuf>,
}

impl DepthCamera {
    pub(crate) fn matches_color_device(&self, device: &Path) -> bool {
        self.color_device.as_deref() == Some(device)
    }

    pub fn validate(&self) -> Result<()> {
        if is_device_path(&self.depth_device)
            && self.color_device.as_deref().is_none_or(is_device_path)
            && self.color_device.as_ref() != Some(&self.depth_device)
        {
            Ok(())
        } else {
            Err(Error::new(
                ErrorKind::InvalidInput,
                "V4L2 depth and color paths must be distinct devices under /dev",
            ))
        }
    }

    /// Read capabilities without starting a capture stream.
    pub fn observe(&self) -> Result<HashMap<String, Value>> {
        if !supports_capture(&self.depth_device, &[*b"Z16 "])? {
            return Err(Error::new(
                ErrorKind::Unsupported,
                "V4L2 device does not offer Z16 capture",
            ));
        }
        let mut properties = HashMap::from([("depth_available".into(), Value::Bool(true))]);
        if let Some(color) = &self.color_device {
            let available = supports_capture(color, &[*b"MJPG", *b"YUYV"]).unwrap_or(false);
            properties.insert("color_available".into(), Value::Bool(available));
        }
        Ok(properties)
    }
}

#[cfg(target_os = "linux")]
fn supports_capture(path: &Path, accepted_formats: &[[u8; 4]]) -> Result<bool> {
    use std::fs::OpenOptions;
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

    // Linux videodev2.h: VIDIOC_ENUM_FMT with a v4l2_fmtdesc of 64 bytes.
    const VIDIOC_ENUM_FMT: u32 = 0xc040_5602;
    const V4L2_BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
    const V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE: u32 = 9;

    #[repr(C)]
    #[derive(Default)]
    struct FormatDescription {
        index: u32,
        buffer_type: u32,
        flags: u32,
        description: [u8; 32],
        pixel_format: u32,
        mbus_code: u32,
        reserved: [u32; 3],
    }
    const _: () = assert!(std::mem::size_of::<FormatDescription>() == 64);

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.file_type().is_char_device() {
        return Ok(false);
    }

    for buffer_type in [
        V4L2_BUF_TYPE_VIDEO_CAPTURE,
        V4L2_BUF_TYPE_VIDEO_CAPTURE_MPLANE,
    ] {
        for index in 0..64 {
            let mut format = FormatDescription {
                index,
                buffer_type,
                ..FormatDescription::default()
            };
            // SAFETY: the file remains open and the initialized descriptor
            // has the exact C layout and lifetime required by the ioctl.
            if unsafe { libc::ioctl(file.as_raw_fd(), VIDIOC_ENUM_FMT as _, &mut format) } < 0 {
                let error = Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EINVAL | libc::ENOTTY) => break,
                    _ => return Err(error),
                }
            }
            if accepted_formats.contains(&format.pixel_format.to_le_bytes()) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

#[cfg(not(target_os = "linux"))]
fn supports_capture(_: &Path, _: &[[u8; 4]]) -> Result<bool> {
    Err(Error::new(ErrorKind::Unsupported, "V4L2 requires Linux"))
}

#[cfg(test)]
mod tests {
    use crate::media::MediaSourceConfig;

    #[test]
    fn usb_color_capture_is_a_media_source_without_a_gateway_path() {
        let source: MediaSourceConfig = serde_yaml::from_str(
            "kind: v4l2_camera
id: gemini-335
device: /dev/v4l/by-path/default-video-index0
width: 640
height: 480
frames_per_second: 30",
        )
        .unwrap();
        assert_eq!(source.id(), "gemini-335");
        assert!(source.validate().is_ok());
        assert!(source.paths().is_empty());
    }
}
