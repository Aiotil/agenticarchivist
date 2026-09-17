//! Common formats through the `image` crate, for platforms without native
//! codecs wired in yet. No HEIC, RAW, PDF, or EXIF fields beyond orientation.

use super::SourceMetadata;
use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageReader, metadata::Orientation};
use std::path::Path;

pub const SUPPORTS_PDF: bool = false;
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "bmp"];

pub fn probe(path: &Path, _kind: super::Kind) -> Result<SourceMetadata> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let media_type = reader.format().map(|f| f.to_mime_type().to_string());
    let mut decoder = reader.into_decoder().context("unsupported image")?;
    let (width, height) = decoder.dimensions();
    let orientation = decoder.orientation().map(Orientation::to_exif).unwrap_or(1);
    Ok(SourceMetadata {
        media_type,
        width,
        height,
        orientation,
        page_count: 1,
        ..Default::default()
    })
}

pub fn write_thumbnail(src: &Path, dest: &Path, max_edge: u32, quality: f32) -> Result<(u32, u32)> {
    let mut decoder = ImageReader::open(src)?
        .with_guessed_format()?
        .into_decoder()?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(decoder)?;
    img.apply_orientation(orientation);
    // Never enlarge, matching ImageIO's thumbnails.
    if img.width().max(img.height()) > max_edge {
        img = img.thumbnail(max_edge, max_edge);
    }
    let thumb = img.to_rgb8();
    let mut out = std::io::BufWriter::new(std::fs::File::create(dest)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, (quality * 100.0) as u8)
        .encode_image(&thumb)?;
    Ok(thumb.dimensions())
}

pub fn render_pdf_page(
    _src: &Path,
    _page: u32,
    _dpi: f64,
    _max_edge: u32,
    _dest: &Path,
    _quality: f32,
) -> Result<(u32, u32)> {
    bail!("PDF import is not available on this platform yet")
}
