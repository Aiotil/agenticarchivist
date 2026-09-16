//! Sidecar prototype: write the archival files for a sample collection, then
//! check them with independent tools and rebuild the catalogue from sidecars.

mod catalog;
mod model;
mod xmp;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use model::Collection;

const META_DIR: &str = "_agenticarchivist";

struct Report {
    lines: Vec<String>,
    failures: usize,
}

impl Report {
    fn check(&mut self, ok: bool, what: impl Into<String>) {
        let what = what.into();
        let mark = if ok { "PASS" } else { "FAIL" };
        println!("{mark} {what}");
        self.lines.push(format!("- {mark} {what}"));
        if !ok {
            self.failures += 1;
        }
    }

    fn note(&mut self, what: impl Into<String>) {
        let what = what.into();
        println!("     {what}");
        self.lines.push(format!("  - {what}"));
    }
}

fn main() -> Result<()> {
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()?;
    let out_root = workspace.join("target/sidecar-spike");
    let _ = std::fs::remove_dir_all(&out_root);
    let collection = model::sample();
    let root = out_root.join(&collection.name);
    std::fs::create_dir_all(root.join(META_DIR).join("derived"))?;

    let mut report = Report {
        lines: vec![],
        failures: 0,
    };
    let result = run(&collection, &root, &mut report);
    if let Err(e) = &result {
        report.check(false, format!("aborted: {e:#}"));
    }
    let summary = format!(
        "# Sidecar prototype run\n\nexiftool {}\n\n{}\n\nResult: {} failure(s)\n",
        tool_version("exiftool", &["-ver"]),
        report.lines.join("\n"),
        report.failures
    );
    std::fs::write(out_root.join("report.md"), &summary)?;
    println!(
        "\nOutput: {}\nReport: {}",
        root.display(),
        out_root.join("report.md").display()
    );
    if report.failures > 0 {
        bail!("{} check(s) failed", report.failures);
    }
    Ok(())
}

fn run(c: &Collection, root: &Path, report: &mut Report) -> Result<()> {
    // Originals: small but real image files.
    let mut original_sha = BTreeMap::new();
    for (n, image) in c.works.iter().flat_map(|w| &w.images).enumerate() {
        let path = root.join(&image.file);
        std::fs::create_dir_all(path.parent().unwrap())?;
        write_original(&path, n as u32)?;
        original_sha.insert(image.file.clone(), sha256_file(&path)?);
    }

    // Sidecars next to each original.
    let all_files: Vec<&str> = c
        .works
        .iter()
        .flat_map(|w| &w.images)
        .map(|i| i.file.as_str())
        .collect();
    let mut sidecar_paths = BTreeMap::new();
    for w in &c.works {
        for image in &w.images {
            let dir = parent_dir(&image.file);
            let in_dir: Vec<&str> = all_files
                .iter()
                .copied()
                .filter(|f| parent_dir(f) == dir)
                .collect();
            let name = xmp::sidecar_name(&image.file, &in_dir);
            let rel = if dir.is_empty() {
                name
            } else {
                format!("{dir}/{name}")
            };
            std::fs::write(
                root.join(&rel),
                xmp::sidecar(c, w, image, &original_sha[&image.file]),
            )?;
            sidecar_paths.insert(image.file.clone(), rel);
        }
    }
    report.check(
        sidecar_paths.len() == all_files.len(),
        format!(
            "wrote a sidecar for each original: {}",
            sidecar_paths
                .values()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ),
    );
    report.check(
        sidecar_paths.values().any(|p| p.ends_with("IMG_0600.tif.xmp"))
            && sidecar_paths.values().any(|p| p.ends_with("IMG_0600.jpg.xmp"))
            && sidecar_paths.values().any(|p| p.ends_with("/IMG_0412.xmp")),
        "names follow Adobe style (IMG_0412.xmp) and keep the extension only when base names collide (IMG_0600.tif.xmp, IMG_0600.jpg.xmp)",
    );

    // Generated JPEGs with embedded XMP.
    let mut derived = vec![];
    for w in &c.works {
        for image in &w.images {
            let sha = &original_sha[&image.file];
            let preview = xmp::embed_in_jpeg(
                &jpeg_bytes(320, 240, 60)?,
                &xmp::preview_packet(c, w, image),
            )?;
            let preview_rel = format!("{META_DIR}/derived/{sha}_preview.jpg");
            std::fs::write(root.join(&preview_rel), preview)?;
            derived.push(preview_rel);
            if image.restoration.is_some() {
                let restored = xmp::embed_in_jpeg(
                    &jpeg_bytes(640, 480, 200)?,
                    &xmp::restored_packet(c, w, image, sha)?,
                )?;
                let restored_rel = format!("{META_DIR}/derived/{sha}_restored.jpg");
                std::fs::write(root.join(&restored_rel), restored)?;
                derived.push(restored_rel);
            }
        }
    }

    // Collection-level files.
    std::fs::write(
        root.join(META_DIR).join("works.vra.xml"),
        catalog::vra_core_xml(c),
    )?;
    std::fs::write(
        root.join(META_DIR).join("catalog.csv"),
        catalog::catalog_csv(c, |f| sidecar_paths[f].clone(), |f| original_sha[f].clone()),
    )?;
    std::fs::write(
        root.join(META_DIR).join("catalog.json"),
        serde_json::to_string_pretty(c)?,
    )?;
    std::fs::write(root.join(META_DIR).join("README.txt"), catalog::README)?;

    let mut manifest_files: Vec<String> = original_sha.keys().cloned().collect();
    manifest_files.extend(sidecar_paths.values().cloned());
    manifest_files.extend(derived.iter().cloned());
    manifest_files.sort();
    let mut manifest = String::new();
    for f in &manifest_files {
        manifest.push_str(&format!("{}  {f}\n", sha256_file(&root.join(f))?));
    }
    let manifest_rel = format!("{META_DIR}/manifest-sha256.txt");
    std::fs::write(root.join(&manifest_rel), manifest)?;

    // Independent checks.
    let xml_files: Vec<String> = sidecar_paths
        .values()
        .cloned()
        .chain([format!("{META_DIR}/works.vra.xml")])
        .collect();
    let xmllint = run_cmd("xmllint", &["--noout"], &xml_files, root)?;
    report.check(
        xmllint.0,
        format!("xmllint: all {} XML files are well-formed", xml_files.len()),
    );

    let sidecars: Vec<String> = sidecar_paths.values().cloned().collect();
    let (_, out, _) = run_cmd(
        "exiftool",
        &["-j", "-a", "-G1", "-validate", "-warning", "-error"],
        &sidecars,
        root,
    )?;
    let validation: Vec<serde_json::Value> =
        serde_json::from_str(&out).context("parsing exiftool JSON")?;
    let problems = warnings(&validation);
    report.check(
        problems.is_empty(),
        "exiftool -validate: no warnings or errors on any sidecar",
    );
    for p in &problems {
        report.note(p.clone());
    }
    let (_, out, _) = run_cmd("exiftool", &["-j", "-a", "-G1", "-struct"], &sidecars, root)?;
    let read: Vec<serde_json::Value> =
        serde_json::from_str(&out).context("parsing exiftool JSON")?;

    let letter = read
        .iter()
        .find(|o| {
            o["SourceFile"]
                .as_str()
                .is_some_and(|s| s.ends_with("IMG_0412.xmp"))
        })
        .context("exiftool output for IMG_0412.xmp")?;
    let letter_json = letter.to_string();
    report.check(
        letter["XMP-dc:Title"] == "1943 Letter to Rose"
            && letter["XMP-dc:Creator"].to_string().contains("Saul Abrams")
            && letter["XMP-dc:Subject"].to_string().contains("paper")
            && letter["XMP-photoshop:DateCreated"]
                .to_string()
                .contains("1943"),
        "exiftool reads Dublin Core title, creator, keywords, and date",
    );
    let artwork = &letter["XMP-iptcExt:ArtworkOrObject"];
    report.check(
        artwork.to_string().contains("ABR-0001")
            && artwork.to_string().contains("1943 Letter to Rose"),
        format!("exiftool reads the IPTC Artwork or Object structure: {artwork}"),
    );
    report.check(
        letter["XMP-iptcExt:DigitalSourceType"]
            .as_str()
            .is_some_and(|s| s.ends_with("/print")),
        "exiftool reads IPTC Digital Source Type (digitised from a print)",
    );
    let aa_keys: Vec<&String> = letter
        .as_object()
        .unwrap()
        .keys()
        .filter(|k| k.to_lowercase().contains("agenticarchivist"))
        .collect();
    report.check(
        letter_json.contains("w-0001") && !aa_keys.is_empty(),
        format!(
            "exiftool reads the AgenticArchivist namespace ({} properties, e.g. {})",
            aa_keys.len(),
            aa_keys.first().map_or("", |k| k.as_str())
        ),
    );
    report.check(
        letter_json.contains("Grüße an alle — Dein Saul"),
        "non-ASCII text survives (German umlauts, em dash)",
    );

    let (_, out, _) = run_cmd("exiftool", &["-j", "-G1"], &derived, root)?;
    let jpeg_read: Vec<serde_json::Value> = serde_json::from_str(&out)?;
    let (_, out, _) = run_cmd(
        "exiftool",
        &["-j", "-G1", "-validate", "-warning", "-error"],
        &derived,
        root,
    )?;
    let jpeg_validation: Vec<serde_json::Value> = serde_json::from_str(&out)?;
    let restored = jpeg_read
        .iter()
        .find(|o| {
            o["SourceFile"]
                .as_str()
                .is_some_and(|s| s.ends_with("_restored.jpg"))
        })
        .context("restored JPEG")?;
    report.check(
        jpeg_read.iter().all(|o| o["XMP-dc:Title"].is_string()),
        format!(
            "exiftool reads XMP embedded in all {} generated JPEGs",
            jpeg_read.len()
        ),
    );
    report.check(
        restored["XMP-iptcExt:DigitalSourceType"]
            .as_str()
            .is_some_and(|s| s.ends_with("compositeWithTrainedAlgorithmicMedia"))
            && restored["XMP-iptcExt:AISystemUsed"] == "Google Gemini",
        "restored image is marked as AI-altered (Digital Source Type, AI System Used)",
    );
    let jpeg_warnings = warnings(&jpeg_validation);
    report.check(
        jpeg_warnings.is_empty(),
        "exiftool -validate: no warnings on generated JPEGs",
    );
    for w in jpeg_warnings {
        report.note(w);
    }

    // Rebuild the catalogue from sidecars only.
    let parsed = read_sidecars(root, &sidecar_paths)?;
    let rebuilt = xmp::rebuild(&parsed)?;
    let mut expected = c.clone();
    for w in &mut expected.works {
        for i in &mut w.images {
            // Stored on the restored JPEG, not in the original's sidecar.
            i.restoration = None;
        }
    }
    report.check(
        rebuilt == expected,
        "the full catalogue rebuilds from sidecars alone, identical to the original",
    );
    if rebuilt != expected {
        report.note(format!("rebuilt: {}", serde_json::to_string(&rebuilt)?));
    }

    // An edit made in another tool.
    let letter_rel = sidecar_paths["1943 Letter to Rose/IMG_0412.tif"].clone();
    let letter_sidecar = root.join(&letter_rel);
    let before = std::fs::read_to_string(&letter_sidecar)?;
    let edited_title = "1943 Letter to Rosa (edited in ExifTool)";
    let (ok, _, err) = run_cmd(
        "exiftool",
        &[
            "-overwrite_original",
            &format!("-XMP-dc:Title={edited_title}"),
        ],
        std::slice::from_ref(&letter_rel),
        root,
    )?;
    let after = std::fs::read_to_string(&letter_sidecar)?;
    let reparsed = xmp::read_props(&after)?;
    let still_rebuilds = xmp::rebuild(&[(parent_dir(&letter_rel).to_string(), reparsed.clone())]);
    report.check(
        ok && xmp::text(&reparsed, xmp::NS_DC, "title").as_deref() == Some(edited_title)
            && still_rebuilds.as_ref().is_ok_and(|r| {
                r.works[0].materials.len() == 2
                    && r.works[0].field_sources.len() == 3
                    && r.works[0].inscription.is_some()
            }),
        format!(
            "after ExifTool rewrites a sidecar, our reader sees the new title and every AgenticArchivist field survives ({} → {} bytes){}",
            before.len(),
            after.len(),
            err.trim()
        ),
    );
    if let Err(e) = &still_rebuilds {
        report.note(format!("rebuild after ExifTool edit failed: {e:#}"));
    }
    std::fs::write(&letter_sidecar, &before)?;

    // Checksums with the standard tool.
    let (ok, out, _) = run_cmd(
        "shasum",
        &["-a", "256", "-c"],
        std::slice::from_ref(&manifest_rel),
        root,
    )?;
    report.check(
        ok,
        format!(
            "shasum -c verifies all {} files in the manifest",
            out.lines().filter(|l| l.ends_with(": OK")).count()
        ),
    );
    let damaged = root.join("1943 Letter to Rose/IMG_0413.tif");
    let good = std::fs::read(&damaged)?;
    let mut bad = good.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0x01;
    std::fs::write(&damaged, &bad)?;
    let (ok_bad, out_bad, _) = run_cmd(
        "shasum",
        &["-a", "256", "-c"],
        std::slice::from_ref(&manifest_rel),
        root,
    )?;
    std::fs::write(&damaged, &good)?;
    report.check(
        !ok_bad && out_bad.contains("IMG_0413.tif: FAILED"),
        "a single flipped bit in an original is caught and named",
    );

    // CSV and JSON through another language's parsers.
    let py = r#"
import csv, json, sys
rows = list(csv.DictReader(open(sys.argv[1], newline='', encoding='utf-8')))
data = json.load(open(sys.argv[2], encoding='utf-8'))
print(len(rows), len(rows[0]) if rows else 0, len(data['works']))
print(next(r['inscription'] for r in rows if r['image_file'].endswith('IMG_0412.tif')))
"#;
    let out = Command::new("python3")
        .arg("-c")
        .arg(py)
        .arg(format!("{META_DIR}/catalog.csv"))
        .arg(format!("{META_DIR}/catalog.json"))
        .current_dir(root)
        .output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let counts = lines.next().unwrap_or_default().to_string();
    let inscription = lines.next().unwrap_or_default().to_string();
    report.check(
        counts == format!("4 {} 2", catalog::CSV_COLUMNS.len()) && inscription.contains("Grüße"),
        format!(
            "Python's csv and json modules read the catalogue ({counts}: rows, columns, works)"
        ),
    );

    let (_, out, _) = run_cmd(
        "xmllint",
        &[
            "--xpath",
            "concat(count(//*[local-name()='work']),' ',count(//*[local-name()='image']))",
        ],
        &[format!("{META_DIR}/works.vra.xml")],
        root,
    )?;
    report.check(
        out.trim() == "2 4",
        format!(
            "works.vra.xml holds 2 work and 4 image records ({})",
            out.trim()
        ),
    );

    let untouched = original_sha
        .iter()
        .all(|(f, sha)| sha256_file(&root.join(f)).is_ok_and(|now| &now == sha));
    report.check(
        untouched,
        "originals are byte-identical to when they were created",
    );

    let sizes: Vec<String> = sidecar_paths
        .values()
        .map(|p| {
            format!(
                "{} {} bytes",
                xmp::file_name(p),
                std::fs::metadata(root.join(p))
                    .map(|m| m.len())
                    .unwrap_or(0)
            )
        })
        .collect();
    report.note(format!("sidecar sizes: {}", sizes.join(", ")));
    Ok(())
}

/// Warnings, errors, and any `-validate` result other than OK.
fn warnings(exiftool_json: &[serde_json::Value]) -> Vec<String> {
    exiftool_json
        .iter()
        .flat_map(|o| {
            let mut found: Vec<String> = o
                .as_object()
                .into_iter()
                .flatten()
                .filter(|(k, _)| k.ends_with(":Warning") || k.ends_with(":Error"))
                .map(|(k, v)| format!("{} {k}={v}", o["SourceFile"]))
                .collect();
            if o["ExifTool:Validate"] != "OK" {
                found.push(format!(
                    "{} Validate={}",
                    o["SourceFile"], o["ExifTool:Validate"]
                ));
            }
            found
        })
        .collect()
}

fn read_sidecars(
    root: &Path,
    sidecar_paths: &BTreeMap<String, String>,
) -> Result<Vec<(String, xmp::Props)>> {
    sidecar_paths
        .values()
        .map(|rel| {
            let xml = std::fs::read_to_string(root.join(rel))?;
            Ok((parent_dir(rel).to_string(), xmp::read_props(&xml)?))
        })
        .collect()
}

fn parent_dir(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(d, _)| d)
}

fn write_original(path: &Path, seed: u32) -> Result<()> {
    match path.extension().and_then(|e| e.to_str()) {
        Some("tif") => {
            let img = image::ImageBuffer::from_fn(640, 480, |x, y| {
                image::Rgb([
                    (x * 100 + seed * 7000) as u16,
                    (y * 130) as u16,
                    ((x + y) * 50) as u16,
                ])
            });
            image::DynamicImage::ImageRgb16(img).save(path)?;
        }
        Some("jpg") => std::fs::write(path, jpeg_bytes(640, 480, 90)?)?,
        other => bail!("unsupported original type {other:?}"),
    }
    Ok(())
}

fn jpeg_bytes(w: u32, h: u32, tone: u8) -> Result<Vec<u8>> {
    let img = image::ImageBuffer::from_fn(w, h, |x, y| {
        image::Rgb([tone, (x % 256) as u8, (y % 256) as u8])
    });
    let mut buf = std::io::Cursor::new(vec![]);
    image::DynamicImage::ImageRgb8(img).write_to(&mut buf, image::ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

fn sha256_file(path: &Path) -> Result<String> {
    Ok(Sha256::digest(std::fs::read(path)?)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn run_cmd(
    program: &str,
    args: &[&str],
    files: &[String],
    cwd: &Path,
) -> Result<(bool, String, String)> {
    let out = Command::new(program)
        .args(args)
        .args(files)
        .current_dir(cwd)
        .output()
        .with_context(|| format!("running {program}"))?;
    Ok((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    ))
}

fn tool_version(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "not installed".into())
}
