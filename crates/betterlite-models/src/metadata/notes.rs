use serde::{Deserialize, Serialize};

// original_pricing_per_hour/original_pricing_per_minute also live here but belong to B204's MetadataPricing.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CatalogNotes {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calculation: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub successor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_tool_choice: Option<bool>,
}

impl CatalogNotes {
    // Lets entry::unknown_paths report "metadata.<key>" for a genuinely new nested field.
    pub(crate) const WIRE_NAMES: &'static [&'static str] = &[
        "calculation",
        "comment",
        "note",
        "notes",
        "successor",
        "supports_tool_choice",
    ];
}
