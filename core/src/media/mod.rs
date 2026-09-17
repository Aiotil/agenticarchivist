//! Reading originals: which files are supported, their source metadata,
//! thumbnails, and PDF page renders.
//!
//! Apple platforms use ImageIO and Core Graphics, which read HEIC, camera RAW,
//! and PDF. Other platforms use the `image` crate for common formats until
//! their native codecs are wired in.

use std::path::Path;

#[cfg(target_vendor = "apple")]
mod apple;
#[cfg(target_vendor = "apple")]
use apple as platform;

#[cfg(not(target_vendor = "apple"))]
mod portable;
#[cfg(not(target_vendor = "apple"))]
use portable as platform;

pub use platform::{probe, render_pdf_page, write_thumbnail};

/// Long edge of a thumbnail, in pixels.
pub const THUMBNAIL_EDGE: u32 = 400;
pub const THUMBNAIL_QUALITY: f32 = 0.80;
/// PDF pages are rendered at this resolution, as the previous watcher did.
pub const PDF_DPI: f64 = 300.0;
pub const PDF_QUALITY: f32 = 0.92;
/// Pages larger than this (posters, plans) are rendered at a lower resolution.
pub const PDF_MAX_EDGE: u32 = 12_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Pdf,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Image => "image",
            Kind::Pdf => "pdf",
        }
    }
}

/// The kind of original a path holds, judged by extension, or `None` if this
/// platform can't import it.
pub fn kind_for(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if ext == "pdf" {
        return platform::SUPPORTS_PDF.then_some(Kind::Pdf);
    }
    platform::IMAGE_EXTENSIONS
        .contains(&ext.as_str())
        .then_some(Kind::Image)
}

/// Metadata read from an original at import.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SourceMetadata {
    /// Uniform type identifier on Apple platforms (`public.heic`), otherwise a MIME type.
    pub media_type: Option<String>,
    /// Stored pixel size, before EXIF orientation. For PDFs, the first page at [`PDF_DPI`].
    pub width: u32,
    pub height: u32,
    /// EXIF orientation, 1–8. PDFs are always 1.
    pub orientation: u8,
    pub page_count: u32,
    /// RFC 3339 capture time from EXIF, with offset when the camera recorded one.
    pub captured_at: Option<String>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub lens_model: Option<String>,
    pub gps_latitude: Option<f64>,
    pub gps_longitude: Option<f64>,
}

impl SourceMetadata {
    /// Width and height as displayed, after EXIF orientation.
    pub fn display_size(&self) -> (u32, u32) {
        if (5..=8).contains(&self.orientation) {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        }
    }
}

/// Trailing number in a file name, e.g. 412 for `IMG_0412.HEIC`, used later to
/// keep captures of one object together.
pub fn sequence_number(path: &Path) -> Option<i64> {
    let stem = path.file_stem()?.to_str()?.trim_end();
    let digits: String = stem
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if digits.is_empty() || digits.len() > 12 {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_numbers() {
        assert_eq!(sequence_number(Path::new("IMG_0412.HEIC")), Some(412));
        assert_eq!(sequence_number(Path::new("Sidney0130.jpg")), Some(130));
        assert_eq!(sequence_number(Path::new("letter.tif")), None);
        assert_eq!(sequence_number(Path::new("scan 7 .png")), Some(7));
    }

    #[test]
    fn kinds_by_extension() {
        assert_eq!(kind_for(Path::new("a/IMG_1.JPG")), Some(Kind::Image));
        assert_eq!(kind_for(Path::new("a/notes.txt")), None);
        assert_eq!(kind_for(Path::new("a/noext")), None);
        #[cfg(target_vendor = "apple")]
        {
            assert_eq!(kind_for(Path::new("x.pdf")), Some(Kind::Pdf));
            assert_eq!(kind_for(Path::new("x.dng")), Some(Kind::Image));
            assert_eq!(kind_for(Path::new("x.HEIC")), Some(Kind::Image));
        }
    }

    #[test]
    fn display_size_swaps_for_rotated_orientations() {
        let m = SourceMetadata {
            width: 60,
            height: 40,
            orientation: 6,
            ..Default::default()
        };
        assert_eq!(m.display_size(), (40, 60));
    }
}
