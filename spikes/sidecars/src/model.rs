//! Catalogue model and sample collection.
//!
//! The sample people, places, and objects are invented. Getty AAT IDs are
//! believed correct but should be checked against the AAT before reuse.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Collection {
    pub id: String,
    pub name: String,
    pub source: String,
    pub rights: String,
    pub works: Vec<Work>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Work {
    pub id: String,
    pub accession: String,
    pub title: String,
    pub description: String,
    pub work_type: Term,
    pub date_display: String,
    pub date_earliest: i32,
    pub date_latest: i32,
    pub circa: bool,
    pub creators: Vec<Agent>,
    pub materials: Vec<Term>,
    pub techniques: Vec<Term>,
    pub subjects: Vec<Term>,
    pub inscription: Option<Inscription>,
    pub measurements: String,
    pub place_created: Option<Place>,
    pub field_sources: Vec<FieldSource>,
    pub images: Vec<ImageRec>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Term {
    pub label: String,
    pub aat_id: Option<String>,
}

impl Term {
    pub fn aat_uri(&self) -> Option<String> {
        self.aat_id
            .as_ref()
            .map(|id| format!("http://vocab.getty.edu/aat/{id}"))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Agent {
    pub name: String,
    pub role: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inscription {
    pub text: String,
    pub position: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Place {
    pub city: String,
    pub country: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FieldSource {
    pub field: String,
    /// `manual` or `ai`.
    pub source: String,
    pub confidence: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ImageRec {
    /// Path relative to the collection folder.
    pub file: String,
    /// `recto`, `verso`, `detail`, or `overall`.
    pub role: String,
    pub order: u32,
    pub restoration: Option<Restoration>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Restoration {
    pub ai_system: String,
    pub ai_system_version: String,
    pub prompt: String,
}

fn term(label: &str, aat: &str) -> Term {
    Term {
        label: label.into(),
        aat_id: Some(aat.into()),
    }
}

fn manual(field: &str) -> FieldSource {
    FieldSource {
        field: field.into(),
        source: "manual".into(),
        confidence: None,
    }
}

fn ai(field: &str, confidence: &str) -> FieldSource {
    FieldSource {
        field: field.into(),
        source: "ai".into(),
        confidence: Some(confidence.into()),
    }
}

pub fn sample() -> Collection {
    Collection {
        id: "c-7f3a9e21".into(),
        name: "Grandpa Saul Letters".into(),
        source: "Abrams family collection".into(),
        rights: "Private family archive. Not for publication.".into(),
        works: vec![
            Work {
                id: "w-0001".into(),
                accession: "ABR-0001".into(),
                title: "1943 Letter to Rose".into(),
                description: "Handwritten letter in German from Saul to his sister Rose, sent from Lisbon.".into(),
                work_type: term("letters (correspondence)", "300026879"),
                date_display: "1943".into(),
                date_earliest: 1943,
                date_latest: 1943,
                circa: false,
                creators: vec![Agent { name: "Saul Abrams".into(), role: "writer".into() }],
                materials: vec![term("ink", "300015012"), term("paper", "300014109")],
                techniques: vec![],
                subjects: vec![Term { label: "emigration".into(), aat_id: None }],
                inscription: Some(Inscription {
                    text: "Liebe Rosa, wir sind gesund und warten auf das Schiff. Grüße an alle — Dein Saul".into(),
                    position: "recto".into(),
                }),
                measurements: "27 × 21 cm".into(),
                place_created: Some(Place { city: "Lisbon".into(), country: "Portugal".into() }),
                field_sources: vec![manual("title"), ai("date", "0.92"), ai("description", "0.81")],
                images: vec![
                    ImageRec { file: "1943 Letter to Rose/IMG_0412.tif".into(), role: "recto".into(), order: 1, restoration: None },
                    ImageRec { file: "1943 Letter to Rose/IMG_0413.tif".into(), role: "verso".into(), order: 2, restoration: None },
                ],
            },
            Work {
                id: "w-0002".into(),
                accession: "ABR-0002".into(),
                title: "Wedding portrait, c. 1950".into(),
                description: "Studio portrait of Saul and Miriam on their wedding day.".into(),
                work_type: term("portraits", "300015637"),
                date_display: "c. 1950".into(),
                date_earliest: 1948,
                date_latest: 1952,
                circa: true,
                creators: vec![Agent { name: "Unknown studio".into(), role: "photographer".into() }],
                materials: vec![],
                techniques: vec![term("gelatin silver prints", "300128695")],
                subjects: vec![Term { label: "weddings".into(), aat_id: None }],
                inscription: Some(Inscription { text: "Studio Kowalski, Brooklyn".into(), position: "verso".into() }),
                measurements: "18 × 13 cm".into(),
                place_created: Some(Place { city: "Brooklyn".into(), country: "United States".into() }),
                field_sources: vec![manual("title"), ai("date", "0.64")],
                images: vec![
                    ImageRec {
                        file: "scans-2026-09-12/IMG_0600.tif".into(),
                        role: "recto".into(),
                        order: 1,
                        restoration: Some(Restoration {
                            ai_system: "Google Gemini".into(),
                            ai_system_version: "gemini-3-pro-image-preview".into(),
                            prompt: "Remove cracks and restore faded tones without altering faces.".into(),
                        }),
                    },
                    // A camera JPEG with the same base name as the TIFF: sidecars must not collide.
                    ImageRec { file: "scans-2026-09-12/IMG_0600.jpg".into(), role: "detail".into(), order: 2, restoration: None },
                ],
            },
        ],
    }
}
