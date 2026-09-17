//! End-to-end import of generated originals into a temporary collection.

use agenticarchivist_core::import::{Options, import_folder, sha256_file};
use agenticarchivist_core::library::Library;
use std::path::Path;
use std::time::Duration;

fn options() -> Options {
    Options {
        workers: 4,
        settle: Duration::ZERO,
    }
}

/// A 60×40 JPEG, optionally with EXIF orientation, capture time, and camera make.
fn jpeg(orientation: Option<u16>, seed: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(60, 40, |x, y| {
        image::Rgb([(x * 4) as u8, (y * 6) as u8, seed])
    });
    let mut buf = std::io::Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 90)
        .encode_image(&img)
        .unwrap();
    let data = buf.into_inner();
    let Some(orientation) = orientation else {
        return data;
    };

    // Big-endian TIFF: IFD0 {Make, Orientation, ExifIFD} → Exif IFD {DateTimeOriginal}.
    let mut t: Vec<u8> = b"MM\0\x2a\0\0\0\x08".to_vec();
    let entry = |t: &mut Vec<u8>, tag: u16, typ: u16, count: u32, value: [u8; 4]| {
        t.extend(tag.to_be_bytes());
        t.extend(typ.to_be_bytes());
        t.extend(count.to_be_bytes());
        t.extend(value);
    };
    let make = b"TestCam\0";
    let date = b"2001:02:03 04:05:06\0";
    let ifd0_end = 8 + 2 + 3 * 12 + 4; // 50
    let exif_ifd = ifd0_end + make.len() as u32; // 58
    let date_at = exif_ifd + 2 + 12 + 4; // 76
    t.extend(3u16.to_be_bytes());
    entry(&mut t, 0x010F, 2, make.len() as u32, ifd0_end.to_be_bytes());
    let mut o = [0u8; 4];
    o[..2].copy_from_slice(&orientation.to_be_bytes());
    entry(&mut t, 0x0112, 3, 1, o);
    entry(&mut t, 0x8769, 4, 1, exif_ifd.to_be_bytes());
    t.extend(0u32.to_be_bytes());
    t.extend(make);
    t.extend(1u16.to_be_bytes());
    entry(&mut t, 0x9003, 2, date.len() as u32, date_at.to_be_bytes());
    t.extend(0u32.to_be_bytes());
    t.extend(date);

    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend(t);
    let mut out = data[..2].to_vec(); // SOI
    out.extend([0xFF, 0xE1]);
    out.extend(((app1.len() + 2) as u16).to_be_bytes());
    out.extend(app1);
    out.extend(&data[2..]);
    out
}

/// A PDF with pages of 72×144 pt; `rotate` gives each page's /Rotate.
#[cfg(target_vendor = "apple")]
fn pdf(rotate: &[u32]) -> Vec<u8> {
    let n = rotate.len();
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        format!(
            "<< /Type /Pages /Kids [{}] /Count {n} >>",
            (0..n)
                .map(|i| format!("{} 0 R", 3 + i))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    ];
    for r in rotate {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 144] /Rotate {r} /Contents {} 0 R >>",
            3 + n
        ));
    }
    let content = "0 0 0 rg 0 0 36 36 re f";
    objects.push(format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    ));

    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
    for o in offsets {
        out.extend(format!("{o:010} 00000 n \n").bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .bytes(),
    );
    out
}

fn image_size(path: &Path) -> (u32, u32) {
    image::image_dimensions(path).unwrap()
}

#[test]
fn imports_photos_skips_unchanged_and_finds_duplicates() {
    let source = tempfile::tempdir().unwrap();
    let collection = tempfile::tempdir().unwrap();
    let s = source.path();
    std::fs::create_dir(s.join("box 1")).unwrap();
    std::fs::write(s.join("box 1/IMG_0412.jpg"), jpeg(Some(6), 1)).unwrap();
    std::fs::write(s.join("IMG_0413.JPG"), jpeg(None, 2)).unwrap();
    std::fs::write(s.join("copy of 0413.jpg"), jpeg(None, 2)).unwrap();
    std::fs::write(s.join("notes.txt"), "not an image").unwrap();
    std::fs::write(s.join(".hidden.jpg"), jpeg(None, 3)).unwrap();
    std::fs::write(s.join("broken.jpg"), vec![0xFFu8; 500]).unwrap();
    std::fs::create_dir(s.join("_agenticarchivist")).unwrap();
    std::fs::write(s.join("_agenticarchivist/thumb.jpg"), jpeg(None, 4)).unwrap();

    let mut lib = Library::open(collection.path()).unwrap();
    let r = import_folder(&mut lib, s, &options(), |_| {}).unwrap();
    assert_eq!(r.counts.found, 4, "{r:?}");
    assert_eq!(r.counts.added, 2, "{r:?}");
    assert_eq!(r.counts.duplicates, 1, "{r:?}");
    assert_eq!(r.counts.failed, 1, "{r:?}");
    assert_eq!(r.unsupported, 1);
    assert!(r.errors[0].0.ends_with("broken.jpg"));

    let sheet = lib.sheet_images().unwrap();
    assert_eq!(sheet.len(), 2);
    let rotated = sheet
        .iter()
        .find(|i| i.file_name == "IMG_0412.jpg")
        .unwrap();
    assert_eq!(rotated.orientation, 6);
    assert_eq!(
        (rotated.width, rotated.height),
        (40, 60),
        "display size follows EXIF orientation"
    );
    assert_eq!(
        rotated.sha256,
        sha256_file(&s.join("box 1/IMG_0412.jpg")).unwrap()
    );
    let thumb = lib.meta_dir().join(&rotated.thumbnail);
    assert_eq!(image_size(&thumb), (40, 60), "thumbnail is turned upright");
    #[cfg(target_vendor = "apple")]
    {
        assert_eq!(rotated.captured_at.as_deref(), Some("2001-02-03T04:05:06"));
        assert_eq!(rotated.captured_at_from, "exif");
        assert_eq!(rotated.camera.as_deref(), Some("TestCam"));
    }
    let plain = sheet
        .iter()
        .find(|i| i.file_name == "IMG_0413.JPG")
        .unwrap();
    assert_eq!(plain.captured_at_from, "file");
    assert_eq!(image_size(&lib.meta_dir().join(&plain.thumbnail)), (60, 40));

    // Source folder untouched: no new files.
    let mut names: Vec<_> = std::fs::read_dir(s)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(names.len(), 7);

    // A second run hashes nothing new.
    let r = import_folder(&mut lib, s, &options(), |_| {}).unwrap();
    assert_eq!(
        (r.counts.added, r.counts.unchanged, r.counts.duplicates),
        (0, 3, 0),
        "{r:?}"
    );
    assert_eq!(r.bytes_hashed, 500, "only the failed file is read again");

    // A copy elsewhere is recorded as another location, not a new original.
    std::fs::copy(
        s.join("box 1/IMG_0412.jpg"),
        s.join("box 1/IMG_0412 copy.jpg"),
    )
    .unwrap();
    let r = import_folder(&mut lib, s, &options(), |_| {}).unwrap();
    assert_eq!((r.counts.added, r.counts.duplicates), (0, 1), "{r:?}");
    let summary = lib.summary().unwrap();
    assert_eq!(
        (
            summary.originals,
            summary.duplicate_locations,
            summary.imports
        ),
        (2, 2, 3)
    );
}

#[test]
fn leaves_files_still_being_written() {
    let source = tempfile::tempdir().unwrap();
    let collection = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("IMG_1.jpg"), jpeg(None, 9)).unwrap();
    let mut lib = Library::open(collection.path()).unwrap();
    let opts = Options {
        workers: 1,
        settle: Duration::from_secs(60),
    };
    let r = import_folder(&mut lib, source.path(), &opts, |_| {}).unwrap();
    assert_eq!((r.counts.added, r.still_writing), (0, 1));
}

#[test]
fn collection_inside_source_is_not_imported() {
    let source = tempfile::tempdir().unwrap();
    let collection = source.path().join("My Collection");
    std::fs::write(source.path().join("IMG_1.jpg"), jpeg(None, 5)).unwrap();
    let mut lib = Library::open(&collection).unwrap();
    import_folder(&mut lib, source.path(), &options(), |_| {}).unwrap();
    // Thumbnails now exist inside the source folder's subtree; they must not be picked up.
    let r = import_folder(&mut lib, source.path(), &options(), |_| {}).unwrap();
    assert_eq!((r.counts.found, r.counts.unchanged), (1, 1), "{r:?}");
}

#[cfg(target_vendor = "apple")]
#[test]
fn renders_every_pdf_page_upright() {
    let source = tempfile::tempdir().unwrap();
    let collection = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("letters.pdf"), pdf(&[0, 90])).unwrap();
    let mut lib = Library::open(collection.path()).unwrap();
    let r = import_folder(&mut lib, source.path(), &options(), |_| {}).unwrap();
    assert_eq!(
        (r.counts.added, r.images_added, r.counts.failed),
        (1, 2, 0),
        "{r:?}"
    );

    let sheet = lib.sheet_images().unwrap();
    assert_eq!(sheet.iter().map(|i| i.page).collect::<Vec<_>>(), [1, 2]);
    // 72×144 pt at 300 DPI is 300×600 px; /Rotate 90 turns it to 600×300.
    assert_eq!((sheet[0].width, sheet[0].height), (300, 600));
    assert_eq!((sheet[1].width, sheet[1].height), (600, 300));
    let dir = lib.meta_dir().join(Library::derived_dir(&sheet[0].sha256));
    assert_eq!(image_size(&dir.join("page-0001.jpg")), (300, 600));
    assert_eq!(image_size(&dir.join("page-0002.jpg")), (600, 300));
    assert_eq!(image_size(&dir.join("page-0001-thumb.jpg")), (200, 400));

    // The black square is drawn at the page's lower left. Unrotated, that's the
    // bottom-left of the render; rotated 90° clockwise, it's the top-left.
    let p1 = image::open(dir.join("page-0001.jpg")).unwrap().to_luma8();
    assert!(p1.get_pixel(20, 580).0[0] < 60 && p1.get_pixel(280, 20).0[0] > 200);
    let p2 = image::open(dir.join("page-0002.jpg")).unwrap().to_luma8();
    assert!(
        p2.get_pixel(20, 20).0[0] < 60,
        "rotated page's square should be top-left"
    );
    assert!(p2.get_pixel(580, 280).0[0] > 200);
}
