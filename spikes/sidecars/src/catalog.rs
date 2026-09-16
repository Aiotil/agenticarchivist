//! Collection-level files: VRA Core XML, CSV, JSON, checksum manifest, README.

use quick_xml::escape::escape;
use std::fmt::Write;

use crate::model::{Collection, Term, Work};
use crate::xmp::file_name;

fn term_xml(tag: &str, t: &Term, type_attr: Option<&str>) -> String {
    let mut attrs = String::new();
    if let Some(ty) = type_attr {
        let _ = write!(attrs, " type=\"{ty}\"");
    }
    if let Some(id) = &t.aat_id {
        let _ = write!(attrs, " vocab=\"AAT\" refid=\"{id}\"");
    }
    format!("<{tag}{attrs}>{}</{tag}>", escape(&t.label))
}

/// VRA Core 4 XML: one `work` record per work and one `image` record per file.
/// Elements inside each record follow VRA Core's alphabetical convention.
pub fn vra_core_xml(c: &Collection) -> String {
    let mut x = String::new();
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str("<vra xmlns=\"http://www.vraweb.org/vracore4.htm\"\n     xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"\n     xsi:schemaLocation=\"http://www.vraweb.org/vracore4.htm http://www.loc.gov/standards/vracore/vra.xsd\">\n");
    for w in &c.works {
        let image_ids: Vec<String> = w.images.iter().map(|i| image_id(w, i.order)).collect();
        let _ = writeln!(
            x,
            "  <work id=\"{}\" refid=\"{}\" source=\"{}\">",
            w.id,
            escape(&w.accession),
            escape(&c.source)
        );

        let _ = writeln!(
            x,
            "    <agentSet>\n      <display>{}</display>",
            escape(
                w.creators
                    .iter()
                    .map(|a| format!("{} ({})", a.name, a.role))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        );
        for a in &w.creators {
            let _ = writeln!(
                x,
                "      <agent>\n        <name type=\"personal\">{}</name>\n        <role>{}</role>\n      </agent>",
                escape(&a.name),
                escape(&a.role)
            );
        }
        x.push_str("    </agentSet>\n");

        let _ = writeln!(
            x,
            "    <dateSet>\n      <display>{}</display>\n      <date type=\"creation\">\n        <earliestDate circa=\"{}\">{}</earliestDate>\n        <latestDate circa=\"{}\">{}</latestDate>\n      </date>\n    </dateSet>",
            escape(&w.date_display),
            w.circa,
            w.date_earliest,
            w.circa,
            w.date_latest
        );
        let _ = writeln!(
            x,
            "    <descriptionSet>\n      <display>{0}</display>\n      <description>{0}</description>\n    </descriptionSet>",
            escape(&w.description)
        );

        if let Some(ins) = &w.inscription {
            let _ = writeln!(
                x,
                "    <inscriptionSet>\n      <display>{0}</display>\n      <inscription>\n        <position>{1}</position>\n        <text type=\"text\">{0}</text>\n      </inscription>\n    </inscriptionSet>",
                escape(&ins.text),
                escape(&ins.position)
            );
        }
        if let Some(place) = &w.place_created {
            let _ = writeln!(
                x,
                "    <locationSet>\n      <display>{0}, {1}</display>\n      <location type=\"creation\">\n        <name type=\"geographic\">{0}, {1}</name>\n      </location>\n    </locationSet>",
                escape(&place.city),
                escape(&place.country)
            );
        }
        if !w.materials.is_empty() {
            let _ = writeln!(
                x,
                "    <materialSet>\n      <display>{}</display>",
                escape(labels(&w.materials))
            );
            for m in &w.materials {
                let _ = writeln!(x, "      {}", term_xml("material", m, Some("medium")));
            }
            x.push_str("    </materialSet>\n");
        }
        let _ = writeln!(
            x,
            "    <measurementsSet>\n      <display>{}</display>\n    </measurementsSet>",
            escape(&w.measurements)
        );

        let _ = writeln!(
            x,
            "    <relationSet>\n      <display>{}</display>",
            escape(
                w.images
                    .iter()
                    .map(|i| file_name(&i.file))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        );
        for (i, id) in w.images.iter().zip(&image_ids) {
            let _ = writeln!(
                x,
                "      <relation type=\"imageIs\" relids=\"{id}\"{}>{}</relation>",
                if i.order == 1 { " pref=\"true\"" } else { "" },
                escape(&i.file)
            );
        }
        x.push_str("    </relationSet>\n");

        let _ = writeln!(
            x,
            "    <rightsSet>\n      <display>{0}</display>\n      <rights type=\"undetermined\">\n        <text>{0}</text>\n      </rights>\n    </rightsSet>",
            escape(&c.rights)
        );
        if !w.subjects.is_empty() {
            let _ = writeln!(
                x,
                "    <subjectSet>\n      <display>{}</display>",
                escape(labels(&w.subjects))
            );
            for s in &w.subjects {
                let _ = writeln!(
                    x,
                    "      <subject>\n        {}\n      </subject>",
                    term_xml("term", s, Some("descriptiveTopic"))
                );
            }
            x.push_str("    </subjectSet>\n");
        }
        if !w.techniques.is_empty() {
            let _ = writeln!(
                x,
                "    <techniqueSet>\n      <display>{}</display>",
                escape(labels(&w.techniques))
            );
            for t in &w.techniques {
                let _ = writeln!(x, "      {}", term_xml("technique", t, None));
            }
            x.push_str("    </techniqueSet>\n");
        }
        let _ = writeln!(
            x,
            "    <titleSet>\n      <display>{0}</display>\n      <title type=\"descriptive\" pref=\"true\">{0}</title>\n    </titleSet>",
            escape(&w.title)
        );
        let _ = writeln!(
            x,
            "    <worktypeSet>\n      <display>{}</display>\n      {}\n    </worktypeSet>",
            escape(&w.work_type.label),
            term_xml("worktype", &w.work_type, None)
        );
        x.push_str("  </work>\n");

        for (i, id) in w.images.iter().zip(&image_ids) {
            let _ = writeln!(
                x,
                "  <image id=\"{id}\" source=\"AgenticArchivist\">\n    <relationSet>\n      <display>{title}</display>\n      <relation type=\"imageOf\" relids=\"{wid}\" pref=\"true\">{title}</relation>\n    </relationSet>\n    <titleSet>\n      <display>{file} ({role})</display>\n      <title type=\"descriptive\">{file} ({role})</title>\n    </titleSet>\n  </image>",
                title = escape(&w.title),
                wid = w.id,
                file = escape(&i.file),
                role = escape(&i.role)
            );
        }
    }
    x.push_str("</vra>\n");
    x
}

fn image_id(w: &Work, order: u32) -> String {
    format!("i-{}-{order}", w.id.trim_start_matches("w-"))
}

fn labels(terms: &[Term]) -> String {
    terms
        .iter()
        .map(|t| t.label.clone())
        .collect::<Vec<_>>()
        .join("; ")
}

fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

pub const CSV_COLUMNS: &[&str] = &[
    "collection",
    "work_id",
    "accession",
    "title",
    "work_type",
    "date_display",
    "date_earliest",
    "date_latest",
    "circa",
    "creators",
    "materials",
    "techniques",
    "subjects",
    "inscription",
    "measurements",
    "description",
    "image_file",
    "image_role",
    "image_order",
    "sidecar_file",
    "original_sha256",
];

/// One row per image; work columns repeat so the file works in any spreadsheet.
pub fn catalog_csv(
    c: &Collection,
    sidecar_of: impl Fn(&str) -> String,
    sha_of: impl Fn(&str) -> String,
) -> String {
    let mut out = CSV_COLUMNS.join(",");
    out.push_str("\r\n");
    for w in &c.works {
        for i in &w.images {
            let row = [
                c.name.clone(),
                w.id.clone(),
                w.accession.clone(),
                w.title.clone(),
                w.work_type.label.clone(),
                w.date_display.clone(),
                w.date_earliest.to_string(),
                w.date_latest.to_string(),
                w.circa.to_string(),
                w.creators
                    .iter()
                    .map(|a| format!("{} ({})", a.name, a.role))
                    .collect::<Vec<_>>()
                    .join("; "),
                labels(&w.materials),
                labels(&w.techniques),
                labels(&w.subjects),
                w.inscription
                    .as_ref()
                    .map(|i| i.text.clone())
                    .unwrap_or_default(),
                w.measurements.clone(),
                w.description.clone(),
                i.file.clone(),
                i.role.clone(),
                i.order.to_string(),
                sidecar_of(&i.file),
                sha_of(&i.file),
            ];
            out.push_str(
                &row.iter()
                    .map(|f| csv_field(f))
                    .collect::<Vec<_>>()
                    .join(","),
            );
            out.push_str("\r\n");
        }
    }
    out
}

pub const README: &str = "\
About this folder
=================

This collection was catalogued with AgenticArchivist, but it does not need
AgenticArchivist to be used. Everything here is in open, standard formats.

Originals
  The image files in the collection folders are the originals. They are never
  modified by AgenticArchivist.

Sidecars (*.xmp next to each original)
  XMP metadata (ISO 16684-1) using Dublin Core, IPTC Photo Metadata, and an
  AgenticArchivist namespace (https://agenticarchivist.com/ns/xmp/1.0/).
  Adobe Lightroom, Adobe Bridge, digiKam, darktable, and ExifTool can read them.
  When two originals share a name (IMG_0600.tif and IMG_0600.jpg), the sidecar
  keeps the full name: IMG_0600.tif.xmp.

_agenticarchivist/works.vra.xml
  The catalogue in VRA Core 4 XML, including which images belong to each work.

_agenticarchivist/catalog.csv and catalog.json
  The same catalogue as a spreadsheet (one row per image) and as JSON.

_agenticarchivist/manifest-sha256.txt
  SHA-256 checksums of every original and sidecar. To check for damage, open a
  terminal in the collection folder and run:
      shasum -a 256 -c _agenticarchivist/manifest-sha256.txt

_agenticarchivist/derived/
  Previews and AI restorations. Restored images are marked as AI-altered in
  their embedded metadata (IPTC Digital Source Type).
";
