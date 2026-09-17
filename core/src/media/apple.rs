//! ImageIO and Core Graphics: every format macOS and iOS can open, including
//! HEIC and camera RAW, plus PDF rendering.

use super::{Kind, PDF_DPI, SourceMetadata};
use anyhow::{Context, Result, anyhow, bail, ensure};
use objc2_core_foundation::{
    CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType, CFURL, CGAffineTransform,
    CGFloat, CGPoint, CGRect, CGSize,
};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext, CGImage,
    CGImageAlphaInfo, CGInterpolationQuality, CGPDFBox, CGPDFDocument, CGPDFPage,
    kCGColorSpaceSRGB,
};
use objc2_image_io::{
    CGImageDestination, CGImageSource, kCGImageDestinationLossyCompressionQuality,
    kCGImageSourceCreateThumbnailFromImageAlways, kCGImageSourceCreateThumbnailWithTransform,
    kCGImageSourceThumbnailMaxPixelSize,
};
use std::path::Path;

pub const SUPPORTS_PDF: bool = true;
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "tif", "tiff", "bmp", "heic", "heif", "avif", "jxl",
    // Camera RAW, decoded by the system's RAW support.
    "dng", "cr2", "cr3", "nef", "nrw", "arw", "raf", "orf", "rw2", "pef", "srw",
];

type Dict = CFDictionary<CFString, CFType>;

fn url(path: &Path) -> Result<CFRetained<CFURL>> {
    CFURL::from_file_path(path).ok_or_else(|| anyhow!("not a valid file path: {}", path.display()))
}

fn typed(d: &CFDictionary) -> &Dict {
    // SAFETY: ImageIO property dictionaries are keyed by strings, and every
    // value is read back through `downcast`.
    unsafe { &*(d as *const CFDictionary).cast::<Dict>() }
}

fn options(pairs: &[(&CFString, &CFType)]) -> CFRetained<Dict> {
    let keys: Vec<&CFString> = pairs.iter().map(|(k, _)| *k).collect();
    let values: Vec<&CFType> = pairs.iter().map(|(_, v)| *v).collect();
    CFDictionary::from_slices(&keys, &values)
}

fn child(d: &Dict, key: &str) -> Option<CFRetained<Dict>> {
    let value = d.get(&CFString::from_str(key))?;
    let dict = value.downcast::<CFDictionary>().ok()?;
    // SAFETY: see `typed`.
    Some(unsafe { CFRetained::cast_unchecked(dict) })
}

fn string(d: &Dict, key: &str) -> Option<String> {
    let s = d
        .get(&CFString::from_str(key))?
        .downcast::<CFString>()
        .ok()?
        .to_string();
    let s = s.trim().trim_end_matches('\0').trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn number(d: &Dict, key: &str) -> Option<CFRetained<CFNumber>> {
    d.get(&CFString::from_str(key))?.downcast::<CFNumber>().ok()
}

pub fn probe(path: &Path, kind: Kind) -> Result<SourceMetadata> {
    match kind {
        Kind::Image => probe_image(path),
        Kind::Pdf => probe_pdf(path),
    }
}

fn open_source(path: &Path) -> Result<CFRetained<CGImageSource>> {
    let url = url(path)?;
    let source = unsafe { CGImageSource::with_url(&url, None) }
        .ok_or_else(|| anyhow!("can't open image"))?;
    ensure!(unsafe { source.count() } > 0, "no readable image in file");
    Ok(source)
}

fn probe_image(path: &Path) -> Result<SourceMetadata> {
    let source = open_source(path)?;
    let index = unsafe { source.primary_image_index() };
    let props = unsafe { source.properties_at_index(index, None) }
        .ok_or_else(|| anyhow!("no image properties"))?;
    let props = typed(&props);
    let int = |d: &Dict, k: &str| number(d, k).and_then(|n| n.as_i64());

    let mut meta = SourceMetadata {
        media_type: unsafe { source.r#type() }.map(|t| t.to_string()),
        width: int(props, "PixelWidth")
            .unwrap_or(0)
            .try_into()
            .unwrap_or(0),
        height: int(props, "PixelHeight")
            .unwrap_or(0)
            .try_into()
            .unwrap_or(0),
        orientation: int(props, "Orientation")
            .filter(|o| (1..=8).contains(o))
            .unwrap_or(1) as u8,
        page_count: 1,
        ..Default::default()
    };
    ensure!(meta.width > 0 && meta.height > 0, "image has no pixel size");

    let tiff = child(props, "{TIFF}");
    if let Some(exif) = child(props, "{Exif}") {
        let taken =
            string(&exif, "DateTimeOriginal").or_else(|| string(&exif, "DateTimeDigitized"));
        let offset = string(&exif, "OffsetTimeOriginal").or_else(|| string(&exif, "OffsetTime"));
        meta.captured_at = taken.and_then(|t| crate::time::exif_to_iso(&t, offset.as_deref()));
        meta.lens_model = string(&exif, "LensModel");
    }
    if let Some(tiff) = &tiff {
        if meta.captured_at.is_none() {
            meta.captured_at =
                string(tiff, "DateTime").and_then(|t| crate::time::exif_to_iso(&t, None));
        }
        meta.camera_make = string(tiff, "Make");
        meta.camera_model = string(tiff, "Model");
    }
    if meta.lens_model.is_none()
        && let Some(aux) = child(props, "{ExifAux}")
    {
        meta.lens_model = string(&aux, "LensModel");
    }
    if let Some(gps) = child(props, "{GPS}") {
        let coord = |value: &str, reference: &str, negative: &str| {
            let v = number(&gps, value)?.as_f64()?;
            let sign = if string(&gps, reference).as_deref() == Some(negative) {
                -1.0
            } else {
                1.0
            };
            Some(sign * v)
        };
        meta.gps_latitude = coord("Latitude", "LatitudeRef", "S");
        meta.gps_longitude = coord("Longitude", "LongitudeRef", "W");
    }
    Ok(meta)
}

fn open_pdf(path: &Path) -> Result<CFRetained<CGPDFDocument>> {
    let url = url(path)?;
    let doc = CGPDFDocument::with_url(Some(&url)).ok_or_else(|| anyhow!("can't open PDF"))?;
    ensure!(
        CGPDFDocument::is_unlocked(Some(&doc)),
        "PDF is password-protected"
    );
    Ok(doc)
}

fn probe_pdf(path: &Path) -> Result<SourceMetadata> {
    let doc = open_pdf(path)?;
    let pages = CGPDFDocument::number_of_pages(Some(&doc));
    ensure!(pages > 0, "PDF has no pages");
    let first = CGPDFDocument::page(Some(&doc), 1).ok_or_else(|| anyhow!("can't read page 1"))?;
    let (w, h, _) = page_geometry(&first, PDF_DPI, u32::MAX)?;
    Ok(SourceMetadata {
        media_type: Some("com.adobe.pdf".into()),
        width: w,
        height: h,
        orientation: 1,
        page_count: pages.try_into().unwrap_or(u32::MAX),
        ..Default::default()
    })
}

/// Pixel size of a page rendered at `dpi` (reduced to fit `max_edge`), and the scale used.
fn page_geometry(page: &CGPDFPage, dpi: f64, max_edge: u32) -> Result<(u32, u32, CGFloat)> {
    let rect = CGPDFPage::box_rect(Some(page), CGPDFBox::CropBox);
    let (mut w, mut h) = (rect.size.width.abs(), rect.size.height.abs());
    ensure!(w >= 1.0 && h >= 1.0, "page has no size");
    if CGPDFPage::rotation_angle(Some(page)).rem_euclid(180) == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    let mut scale = dpi / 72.0;
    let longest = w.max(h) * scale;
    if longest > f64::from(max_edge) {
        scale *= f64::from(max_edge) / longest;
    }
    Ok((
        (w * scale).round() as u32,
        (h * scale).round() as u32,
        scale,
    ))
}

pub fn render_pdf_page(
    src: &Path,
    page_number: u32,
    dpi: f64,
    max_edge: u32,
    dest: &Path,
    quality: f32,
) -> Result<(u32, u32)> {
    let doc = open_pdf(src)?;
    let page = CGPDFDocument::page(Some(&doc), page_number as usize)
        .ok_or_else(|| anyhow!("can't read page {page_number}"))?;
    let (width, height, scale) = page_geometry(&page, dpi, max_edge)?;

    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or_else(|| anyhow!("no sRGB colour space"))?;
    let ctx = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            width as usize,
            height as usize,
            8,
            0,
            Some(&space),
            CGImageAlphaInfo::NoneSkipLast.0,
        )
    }
    .ok_or_else(|| anyhow!("can't allocate a {width}×{height} page"))?;
    let c = Some(&*ctx);
    CGContext::set_rgb_fill_color(c, 1.0, 1.0, 1.0, 1.0);
    CGContext::fill_rect(
        c,
        CGRect::new(
            CGPoint::new(0.0, 0.0),
            CGSize::new(width.into(), height.into()),
        ),
    );
    CGContext::set_interpolation_quality(c, CGInterpolationQuality::High);
    CGContext::scale_ctm(c, scale, scale);
    CGContext::concat_ctm(c, page_transform(&page));
    CGContext::draw_pdf_page(c, Some(&page));

    let image = CGBitmapContextCreateImage(c).ok_or_else(|| anyhow!("can't finish page image"))?;
    write_jpeg(&image, dest, quality)?;
    Ok((width, height))
}

/// Maps page space to an upright output in points: moves the crop box to the
/// origin and applies the page's /Rotate (clockwise, in multiples of 90°).
fn page_transform(page: &CGPDFPage) -> CGAffineTransform {
    let r = CGPDFPage::box_rect(Some(page), CGPDFBox::CropBox);
    let (bx, by, bw, bh) = (r.origin.x, r.origin.y, r.size.width, r.size.height);
    let t = |a, b, c, d, tx, ty| CGAffineTransform { a, b, c, d, tx, ty };
    match CGPDFPage::rotation_angle(Some(page)).rem_euclid(360) {
        90 => t(0.0, -1.0, 1.0, 0.0, -by, bx + bw),
        180 => t(-1.0, 0.0, 0.0, -1.0, bx + bw, by + bh),
        270 => t(0.0, 1.0, -1.0, 0.0, by + bh, -bx),
        _ => t(1.0, 0.0, 0.0, 1.0, -bx, -by),
    }
}

pub fn write_thumbnail(src: &Path, dest: &Path, max_edge: u32, quality: f32) -> Result<(u32, u32)> {
    let source = open_source(src)?;
    let index = unsafe { source.primary_image_index() };
    let max = CFNumber::new_i64(i64::from(max_edge));
    let yes: &CFType = CFBoolean::new(true);
    let opts = options(&[
        (unsafe { kCGImageSourceCreateThumbnailFromImageAlways }, yes),
        (unsafe { kCGImageSourceCreateThumbnailWithTransform }, yes),
        (unsafe { kCGImageSourceThumbnailMaxPixelSize }, &max),
    ]);
    let thumb = unsafe { source.thumbnail_at_index(index, Some(opts.as_opaque())) }
        .ok_or_else(|| anyhow!("can't decode image for thumbnail"))?;
    write_jpeg(&thumb, dest, quality)?;
    Ok((
        CGImage::width(Some(&thumb)) as u32,
        CGImage::height(Some(&thumb)) as u32,
    ))
}

fn write_jpeg(image: &CGImage, dest: &Path, quality: f32) -> Result<()> {
    let uti = CFString::from_static_str("public.jpeg");
    let url = url(dest)?;
    let destination = unsafe { CGImageDestination::with_url(&url, &uti, 1, None) }
        .ok_or_else(|| anyhow!("can't create {}", dest.display()))?;
    let q = CFNumber::new_f64(f64::from(quality));
    let props = options(&[(unsafe { kCGImageDestinationLossyCompressionQuality }, &q)]);
    unsafe { destination.add_image(image, Some(props.as_opaque())) };
    if !unsafe { destination.finalize() } {
        bail!("can't write {}", dest.display());
    }
    std::fs::metadata(dest).with_context(|| format!("{} was not written", dest.display()))?;
    Ok(())
}
