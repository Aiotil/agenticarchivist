//! XMP sidecars: writing, embedding in generated JPEGs, and reading back.

use anyhow::{Result, anyhow, bail};
use quick_xml::escape::escape;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use std::collections::BTreeMap;

use crate::model::{Agent, Collection, FieldSource, ImageRec, Inscription, Place, Term, Work};

pub const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";
pub const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
pub const NS_PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
pub const NS_IPTC_EXT: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";
pub const NS_AA: &str = "https://agenticarchivist.com/ns/xmp/1.0/";

const DST_PRINT: &str = "http://cv.iptc.org/newscodes/digitalsourcetype/print";
const DST_AI_COMPOSITE: &str =
    "http://cv.iptc.org/newscodes/digitalsourcetype/compositeWithTrainedAlgorithmicMedia";

/// Builds an XMP packet one property at a time.
struct Packet {
    body: String,
}

impl Packet {
    fn new() -> Self {
        Self {
            body: String::new(),
        }
    }

    fn text(&mut self, name: &str, value: &str) {
        self.body
            .push_str(&format!("   <{name}>{}</{name}>\n", escape(value)));
    }

    fn alt(&mut self, name: &str, value: &str) {
        self.body.push_str(&format!(
            "   <{name}>\n    <rdf:Alt>\n     <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n    </rdf:Alt>\n   </{name}>\n",
            escape(value)
        ));
    }

    fn list(&mut self, name: &str, kind: &str, values: &[String]) {
        if values.is_empty() {
            return;
        }
        self.body
            .push_str(&format!("   <{name}>\n    <rdf:{kind}>\n"));
        for v in values {
            self.body
                .push_str(&format!("     <rdf:li>{}</rdf:li>\n", escape(v)));
        }
        self.body
            .push_str(&format!("    </rdf:{kind}>\n   </{name}>\n"));
    }

    /// A list of structures; each item is a list of (property, value, is_lang_alt).
    fn struct_list(&mut self, name: &str, kind: &str, items: &[Vec<(&str, String, bool)>]) {
        if items.is_empty() {
            return;
        }
        self.body
            .push_str(&format!("   <{name}>\n    <rdf:{kind}>\n"));
        for fields in items {
            self.body
                .push_str("     <rdf:li rdf:parseType=\"Resource\">\n");
            for (prop, value, lang_alt) in fields {
                if *lang_alt {
                    self.body.push_str(&format!(
                        "      <{prop}>\n       <rdf:Alt>\n        <rdf:li xml:lang=\"x-default\">{}</rdf:li>\n       </rdf:Alt>\n      </{prop}>\n",
                        escape(value)
                    ));
                } else {
                    self.body
                        .push_str(&format!("      <{prop}>{}</{prop}>\n", escape(value)));
                }
            }
            self.body.push_str("     </rdf:li>\n");
        }
        self.body
            .push_str(&format!("    </rdf:{kind}>\n   </{name}>\n"));
    }

    fn finish(self) -> String {
        format!(
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"AgenticArchivist sidecar spike\">\n\
 <rdf:RDF xmlns:rdf=\"{NS_RDF}\">\n\
  <rdf:Description rdf:about=\"\"\n\
    xmlns:dc=\"{NS_DC}\"\n\
    xmlns:xmp=\"{NS_XMP}\"\n\
    xmlns:photoshop=\"{NS_PHOTOSHOP}\"\n\
    xmlns:Iptc4xmpExt=\"{NS_IPTC_EXT}\"\n\
    xmlns:agenticArchivist=\"{NS_AA}\">\n\
{}  </rdf:Description>\n\
 </rdf:RDF>\n\
</x:xmpmeta>\n\
<?xpacket end=\"w\"?>\n",
            self.body
        )
    }
}

fn term_items(work: &Work) -> Vec<Vec<(&'static str, String, bool)>> {
    let mut items = vec![];
    let mut add = |category: &str, t: &Term| {
        let mut fields = vec![
            ("agenticArchivist:category", category.to_string(), false),
            ("agenticArchivist:label", t.label.clone(), false),
        ];
        if let Some(uri) = t.aat_uri() {
            fields.push(("agenticArchivist:aatUri", uri, false));
        }
        items.push(fields);
    };
    add("workType", &work.work_type);
    for t in &work.materials {
        add("material", t);
    }
    for t in &work.techniques {
        add("technique", t);
    }
    for t in &work.subjects {
        add("subject", t);
    }
    items
}

/// Full sidecar for an original file.
pub fn sidecar(
    collection: &Collection,
    work: &Work,
    image: &ImageRec,
    original_sha256: &str,
) -> String {
    let mut p = Packet::new();

    // Dublin Core and Photoshop fields: what photo organisers show by default.
    p.alt("dc:title", &work.title);
    p.alt("dc:description", &work.description);
    p.list(
        "dc:creator",
        "Seq",
        &work
            .creators
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>(),
    );
    let mut keywords: Vec<String> = vec![work.work_type.label.clone()];
    keywords.extend(
        work.materials
            .iter()
            .chain(&work.techniques)
            .chain(&work.subjects)
            .map(|t| t.label.clone()),
    );
    p.list("dc:subject", "Bag", &keywords);
    p.list(
        "dc:type",
        "Bag",
        std::slice::from_ref(&work.work_type.label),
    );
    p.alt("dc:rights", &collection.rights);
    p.text("dc:source", &collection.source);
    p.text("dc:identifier", &work.accession);
    if !work.circa && work.date_earliest == work.date_latest {
        p.text("photoshop:DateCreated", &work.date_earliest.to_string());
    }
    if let Some(place) = &work.place_created {
        p.text("photoshop:City", &place.city);
        p.text("photoshop:Country", &place.country);
    }

    // IPTC Extension: the photographed object itself.
    let mut artwork = vec![
        ("Iptc4xmpExt:AOTitle", work.title.clone(), true),
        (
            "Iptc4xmpExt:AOContentDescription",
            work.description.clone(),
            true,
        ),
        ("Iptc4xmpExt:AOSource", collection.source.clone(), false),
        ("Iptc4xmpExt:AOSourceInvNo", work.accession.clone(), false),
        (
            "Iptc4xmpExt:AOPhysicalDescription",
            work.measurements.clone(),
            true,
        ),
    ];
    if !work.circa && work.date_earliest == work.date_latest {
        artwork.push((
            "Iptc4xmpExt:AODateCreated",
            work.date_earliest.to_string(),
            false,
        ));
    } else {
        artwork.push((
            "Iptc4xmpExt:AOCircaDateCreated",
            work.date_display.clone(),
            false,
        ));
    }
    p.struct_list("Iptc4xmpExt:ArtworkOrObject", "Bag", &[artwork]);
    if let Some(place) = &work.place_created {
        p.struct_list(
            "Iptc4xmpExt:LocationCreated",
            "Bag",
            &[vec![
                ("Iptc4xmpExt:City", place.city.clone(), false),
                ("Iptc4xmpExt:CountryName", place.country.clone(), false),
            ]],
        );
    }
    p.text("Iptc4xmpExt:DigitalSourceType", DST_PRINT);

    // AgenticArchivist fields: everything needed to rebuild the catalogue.
    p.text("agenticArchivist:metadataVersion", "1");
    p.text("agenticArchivist:collectionId", &collection.id);
    p.text("agenticArchivist:collectionName", &collection.name);
    p.text("agenticArchivist:workId", &work.id);
    p.text("agenticArchivist:originalFile", file_name(&image.file));
    p.text("agenticArchivist:originalSha256", original_sha256);
    p.text("agenticArchivist:imageRole", &image.role);
    p.text("agenticArchivist:imageOrder", &image.order.to_string());
    p.text("agenticArchivist:dateDisplay", &work.date_display);
    p.text(
        "agenticArchivist:dateEarliest",
        &work.date_earliest.to_string(),
    );
    p.text("agenticArchivist:dateLatest", &work.date_latest.to_string());
    p.text(
        "agenticArchivist:dateCirca",
        if work.circa { "True" } else { "False" },
    );
    p.text("agenticArchivist:measurements", &work.measurements);
    p.struct_list(
        "agenticArchivist:creators",
        "Seq",
        &work
            .creators
            .iter()
            .map(|a| {
                vec![
                    ("agenticArchivist:name", a.name.clone(), false),
                    ("agenticArchivist:role", a.role.clone(), false),
                ]
            })
            .collect::<Vec<_>>(),
    );
    p.struct_list("agenticArchivist:terms", "Bag", &term_items(work));
    if let Some(ins) = &work.inscription {
        p.struct_list(
            "agenticArchivist:inscriptions",
            "Bag",
            &[vec![
                ("agenticArchivist:text", ins.text.clone(), false),
                ("agenticArchivist:position", ins.position.clone(), false),
            ]],
        );
    }
    if let Some(place) = &work.place_created {
        p.struct_list(
            "agenticArchivist:placeCreated",
            "Bag",
            &[vec![
                ("agenticArchivist:city", place.city.clone(), false),
                ("agenticArchivist:country", place.country.clone(), false),
            ]],
        );
    }
    p.struct_list(
        "agenticArchivist:fieldSources",
        "Bag",
        &work
            .field_sources
            .iter()
            .map(|f| {
                let mut fields = vec![
                    ("agenticArchivist:field", f.field.clone(), false),
                    ("agenticArchivist:source", f.source.clone(), false),
                ];
                if let Some(c) = &f.confidence {
                    fields.push(("agenticArchivist:confidence", c.clone(), false));
                }
                fields
            })
            .collect::<Vec<_>>(),
    );
    p.finish()
}

/// Small packet embedded in a generated preview JPEG.
pub fn preview_packet(collection: &Collection, work: &Work, image: &ImageRec) -> String {
    let mut p = Packet::new();
    p.alt("dc:title", &work.title);
    p.alt("dc:rights", &collection.rights);
    p.text("dc:identifier", &work.accession);
    p.text("agenticArchivist:collectionName", &collection.name);
    p.text("agenticArchivist:workId", &work.id);
    p.text("agenticArchivist:originalFile", &image.file);
    p.text("agenticArchivist:dateDisplay", &work.date_display);
    p.finish()
}

/// Packet embedded in an AI-restored JPEG, marking it as AI-altered.
pub fn restored_packet(
    collection: &Collection,
    work: &Work,
    image: &ImageRec,
    original_sha256: &str,
) -> Result<String> {
    let r = image
        .restoration
        .as_ref()
        .ok_or_else(|| anyhow!("image has no restoration"))?;
    let mut p = Packet::new();
    p.alt("dc:title", &format!("{} (AI restored)", work.title));
    p.alt("dc:rights", &collection.rights);
    p.text("Iptc4xmpExt:DigitalSourceType", DST_AI_COMPOSITE);
    p.text("Iptc4xmpExt:AISystemUsed", &r.ai_system);
    p.text("Iptc4xmpExt:AISystemVersionUsed", &r.ai_system_version);
    p.text("Iptc4xmpExt:AIPromptInformation", &r.prompt);
    p.text("agenticArchivist:workId", &work.id);
    p.text("agenticArchivist:derivedFromFile", &image.file);
    p.text("agenticArchivist:derivedFromSha256", original_sha256);
    Ok(p.finish())
}

/// Insert an XMP packet as an APP1 segment right after the JPEG SOI marker.
pub fn embed_in_jpeg(jpeg: &[u8], packet: &str) -> Result<Vec<u8>> {
    const HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    if jpeg.len() < 2 || jpeg[0..2] != [0xFF, 0xD8] {
        bail!("not a JPEG");
    }
    let len = 2 + HEADER.len() + packet.len();
    if len > 0xFFFF {
        bail!("XMP packet too large for one APP1 segment");
    }
    let mut out = Vec::with_capacity(jpeg.len() + len + 2);
    out.extend_from_slice(&jpeg[0..2]);
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&(len as u16).to_be_bytes());
    out.extend_from_slice(HEADER);
    out.extend_from_slice(packet.as_bytes());
    out.extend_from_slice(&jpeg[2..]);
    Ok(out)
}

/// Sidecar file name for an original, avoiding collisions between originals
/// that share a base name (IMG_0600.tif and IMG_0600.jpg).
pub fn sidecar_name(file: &str, all_files_in_dir: &[&str]) -> String {
    let name = file_name(file);
    let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
    let clashes = all_files_in_dir
        .iter()
        .filter(|f| {
            let n = file_name(f);
            n.rsplit_once('.').map_or(n, |(s, _)| s) == stem
        })
        .count();
    if clashes > 1 {
        format!("{name}.xmp")
    } else {
        format!("{stem}.xmp")
    }
}

pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// A parsed XMP value, independent of how the writer serialised it.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Alt(Vec<(String, String)>),
    List(Vec<Value>),
    Struct(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_text(&self) -> Option<String> {
        match self {
            Value::Text(t) => Some(t.clone()),
            Value::Alt(items) => items
                .iter()
                .find(|(lang, _)| lang == "x-default")
                .or(items.first())
                .map(|(_, v)| v.clone()),
            _ => None,
        }
    }

    pub fn items(&self) -> Vec<Value> {
        match self {
            Value::List(items) => items.clone(),
            other => vec![other.clone()],
        }
    }

    pub fn field(&self, ns: &str, local: &str) -> Option<String> {
        match self {
            Value::Struct(map) => map.get(&format!("{ns}{local}")).and_then(Value::as_text),
            _ => None,
        }
    }
}

pub type Props = BTreeMap<String, Value>;

#[derive(Debug)]
struct Element {
    key: String,
    attrs: Vec<(String, String)>,
    children: Vec<Element>,
    text: String,
}

fn resolved_key(ns: ResolveResult, local: &str) -> String {
    match ns {
        ResolveResult::Bound(ns) => format!("{}{local}", ns.as_ref()),
        _ => local.to_string(),
    }
}

fn parse_dom(xml: &str) -> Result<Element> {
    let mut reader = quick_xml::reader::NsReader::from_str(xml);
    // Do not trim text events: an entity such as &amp; splits the text, and
    // trimming each piece would drop the spaces around it.
    let mut stack: Vec<Element> = vec![Element {
        key: "#root".into(),
        attrs: vec![],
        children: vec![],
        text: String::new(),
    }];
    loop {
        let (ns, event) = reader.read_resolved_event()?;
        let is_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let key = resolved_key(ns, e.local_name().as_ref());
                let mut attrs = vec![];
                for attr in e.attributes() {
                    let attr = attr?;
                    let (ans, alocal) = reader.resolver().resolve_attribute(attr.key);
                    let raw = attr.key.as_ref().to_string();
                    let akey = if let Some(local) = raw.strip_prefix("xml:") {
                        format!("{NS_XML}{local}")
                    } else {
                        resolved_key(ans, alocal.as_ref())
                    };
                    let value = attr
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)?
                        .to_string();
                    attrs.push((akey, value));
                }
                let el = Element {
                    key,
                    attrs,
                    children: vec![],
                    text: String::new(),
                };
                if is_empty {
                    stack.last_mut().unwrap().children.push(el);
                } else {
                    stack.push(el);
                }
            }
            Event::End(_) => {
                let el = stack.pop().ok_or_else(|| anyhow!("unbalanced XML"))?;
                stack
                    .last_mut()
                    .ok_or_else(|| anyhow!("unbalanced XML"))?
                    .children
                    .push(el);
            }
            Event::Text(t) => {
                stack.last_mut().unwrap().text.push_str(&t.xml10_content());
            }
            Event::GeneralRef(r) => {
                let text = if let Some(ch) = r.resolve_char_ref()? {
                    ch.to_string()
                } else {
                    match r.xml10_content().as_ref() {
                        "amp" => "&".into(),
                        "lt" => "<".into(),
                        "gt" => ">".into(),
                        "quot" => "\"".into(),
                        "apos" => "'".into(),
                        other => bail!("unknown entity &{other};"),
                    }
                };
                stack.last_mut().unwrap().text.push_str(&text);
            }
            Event::CData(c) => {
                stack.last_mut().unwrap().text.push_str(c.as_ref());
            }
            Event::Eof => break,
            _ => {}
        }
    }
    stack.pop().ok_or_else(|| anyhow!("empty XML"))
}

fn attr<'a>(el: &'a Element, key: &str) -> Option<&'a str> {
    el.attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn rdf(local: &str) -> String {
    format!("{NS_RDF}{local}")
}

fn parse_value(el: &Element) -> Value {
    if attr(el, &rdf("parseType")) == Some("Resource") {
        return Value::Struct(
            el.children
                .iter()
                .map(|c| (c.key.clone(), parse_value(c)))
                .collect(),
        );
    }
    if let Some(first) = el.children.first() {
        if first.key == rdf("Alt") {
            return Value::Alt(
                first
                    .children
                    .iter()
                    .map(|li| {
                        (
                            attr(li, &format!("{NS_XML}lang"))
                                .unwrap_or("x-default")
                                .to_string(),
                            li.text.trim().to_string(),
                        )
                    })
                    .collect(),
            );
        }
        if first.key == rdf("Seq") || first.key == rdf("Bag") {
            return Value::List(first.children.iter().map(parse_value).collect());
        }
        if first.key == rdf("Description") {
            return description_struct(first);
        }
        return Value::Struct(
            el.children
                .iter()
                .map(|c| (c.key.clone(), parse_value(c)))
                .collect(),
        );
    }
    Value::Text(el.text.trim().to_string())
}

fn description_struct(desc: &Element) -> Value {
    let mut map: BTreeMap<String, Value> = desc
        .attrs
        .iter()
        .filter(|(k, _)| !k.starts_with(NS_RDF) && !k.starts_with(NS_XML) && k.contains("://"))
        .map(|(k, v)| (k.clone(), Value::Text(v.clone())))
        .collect();
    for child in &desc.children {
        map.insert(child.key.clone(), parse_value(child));
    }
    Value::Struct(map)
}

/// Read every property from every rdf:Description in an XMP packet.
pub fn read_props(xml: &str) -> Result<Props> {
    let root = parse_dom(xml)?;
    let mut props = Props::new();
    fn collect(el: &Element, props: &mut Props) {
        if el.key == rdf("Description") {
            if let Value::Struct(map) = description_struct(el) {
                props.extend(map);
            }
            return;
        }
        for c in &el.children {
            collect(c, props);
        }
    }
    collect(&root, &mut props);
    Ok(props)
}

pub fn text(props: &Props, ns: &str, local: &str) -> Option<String> {
    props.get(&format!("{ns}{local}")).and_then(Value::as_text)
}

fn list(props: &Props, ns: &str, local: &str) -> Vec<Value> {
    props
        .get(&format!("{ns}{local}"))
        .map(Value::items)
        .unwrap_or_default()
}

/// Rebuild the catalogue from sidecars alone. `sidecars` pairs each sidecar's
/// folder (relative to the collection) with its parsed properties.
pub fn rebuild(sidecars: &[(String, Props)]) -> Result<Collection> {
    let first = &sidecars.first().ok_or_else(|| anyhow!("no sidecars"))?.1;
    let need = |p: &Props, local: &str| {
        text(p, NS_AA, local).ok_or_else(|| anyhow!("sidecar missing agenticArchivist:{local}"))
    };
    let mut works: BTreeMap<String, Work> = BTreeMap::new();

    for (dir, p) in sidecars {
        let work_id = need(p, "workId")?;
        let original = need(p, "originalFile")?;
        let image = ImageRec {
            file: if dir.is_empty() {
                original
            } else {
                format!("{dir}/{original}")
            },
            role: need(p, "imageRole")?,
            order: need(p, "imageOrder")?.parse()?,
            restoration: None,
        };
        if let Some(work) = works.get_mut(&work_id) {
            work.images.push(image);
            continue;
        }

        let mut work_type = None;
        let (mut materials, mut techniques, mut subjects) = (vec![], vec![], vec![]);
        for item in list(p, NS_AA, "terms") {
            let label = item.field(NS_AA, "label").unwrap_or_default();
            let aat_id = item
                .field(NS_AA, "aatUri")
                .and_then(|u| u.rsplit('/').next().map(str::to_string));
            let t = Term { label, aat_id };
            match item.field(NS_AA, "category").as_deref() {
                Some("workType") => work_type = Some(t),
                Some("material") => materials.push(t),
                Some("technique") => techniques.push(t),
                Some("subject") => subjects.push(t),
                _ => {}
            }
        }
        let artwork = list(p, NS_IPTC_EXT, "ArtworkOrObject");
        let accession = artwork
            .first()
            .and_then(|a| a.field(NS_IPTC_EXT, "AOSourceInvNo"))
            .or_else(|| text(p, NS_DC, "identifier"))
            .unwrap_or_default();

        works.insert(
            work_id.clone(),
            Work {
                id: work_id,
                accession,
                title: text(p, NS_DC, "title").unwrap_or_default(),
                description: text(p, NS_DC, "description").unwrap_or_default(),
                work_type: work_type.ok_or_else(|| anyhow!("no work type"))?,
                date_display: need(p, "dateDisplay")?,
                date_earliest: need(p, "dateEarliest")?.parse()?,
                date_latest: need(p, "dateLatest")?.parse()?,
                circa: need(p, "dateCirca")? == "True",
                creators: list(p, NS_AA, "creators")
                    .iter()
                    .map(|c| Agent {
                        name: c.field(NS_AA, "name").unwrap_or_default(),
                        role: c.field(NS_AA, "role").unwrap_or_default(),
                    })
                    .collect(),
                materials,
                techniques,
                subjects,
                inscription: list(p, NS_AA, "inscriptions").first().map(|i| Inscription {
                    text: i.field(NS_AA, "text").unwrap_or_default(),
                    position: i.field(NS_AA, "position").unwrap_or_default(),
                }),
                measurements: need(p, "measurements")?,
                place_created: list(p, NS_AA, "placeCreated").first().map(|pl| Place {
                    city: pl.field(NS_AA, "city").unwrap_or_default(),
                    country: pl.field(NS_AA, "country").unwrap_or_default(),
                }),
                field_sources: list(p, NS_AA, "fieldSources")
                    .iter()
                    .map(|f| FieldSource {
                        field: f.field(NS_AA, "field").unwrap_or_default(),
                        source: f.field(NS_AA, "source").unwrap_or_default(),
                        confidence: f.field(NS_AA, "confidence"),
                    })
                    .collect(),
                images: vec![image],
            },
        );
    }

    let mut works: Vec<Work> = works.into_values().collect();
    for w in &mut works {
        w.images.sort_by_key(|i| i.order);
    }
    Ok(Collection {
        id: need(first, "collectionId")?,
        name: need(first, "collectionName")?,
        source: text(first, NS_DC, "source").unwrap_or_default(),
        rights: text(first, NS_DC, "rights").unwrap_or_default(),
        works,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::sample;

    #[test]
    fn sidecar_names_avoid_collisions() {
        let dir = ["s/IMG_0600.tif", "s/IMG_0600.jpg", "s/IMG_0601.tif"];
        assert_eq!(sidecar_name("s/IMG_0600.tif", &dir), "IMG_0600.tif.xmp");
        assert_eq!(sidecar_name("s/IMG_0600.jpg", &dir), "IMG_0600.jpg.xmp");
        assert_eq!(sidecar_name("s/IMG_0601.tif", &dir), "IMG_0601.xmp");
    }

    #[test]
    fn catalogue_round_trips_through_sidecars() {
        let c = sample();
        let mut parsed = vec![];
        for w in &c.works {
            for i in &w.images {
                let xml = sidecar(&c, w, i, "00");
                let dir = i.file.rsplit_once('/').map_or("", |(d, _)| d).to_string();
                parsed.push((dir, read_props(&xml).unwrap()));
            }
        }
        let mut expected = c.clone();
        for w in &mut expected.works {
            for i in &mut w.images {
                i.restoration = None;
            }
        }
        assert_eq!(rebuild(&parsed).unwrap(), expected);
    }

    #[test]
    fn reader_handles_attribute_shorthand_and_entities() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
            <rdf:Description rdf:about="" xmlns:aa="https://agenticarchivist.com/ns/xmp/1.0/" aa:workId="w-9"
                xmlns:dc="http://purl.org/dc/elements/1.1/">
              <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Fish &amp; Chips &#x2014; 1950</rdf:li></rdf:Alt></dc:title>
            </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let props = read_props(xml).unwrap();
        assert_eq!(text(&props, NS_AA, "workId").as_deref(), Some("w-9"));
        assert_eq!(
            text(&props, NS_DC, "title").as_deref(),
            Some("Fish & Chips — 1950")
        );
    }
}
