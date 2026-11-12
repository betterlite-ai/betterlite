use betterlite_models::call_helpers::headers::AdditionalHeaders;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

pub type NativeObject = IndexMap<String, Json>;

// Known fields come from the typed parse and the rest from an open parse, since serde's flatten breaks numbers under arbitrary_precision.
pub trait NativeRecord: DeserializeOwned {
    const KNOWN_FIELDS: &'static [&'static str];

    fn set_extension(&mut self, extension: NativeObject);
}

#[expect(
    clippy::unnecessary_wraps,
    reason = "serde's default hook must return the field's Option type"
)]
fn default_object() -> Option<String> {
    Some("interaction".to_owned())
}

// Native content stays open: steps, outputs, usage and any field the API adds travel as given.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct InteractionRecord {
    pub id: Option<String>,
    #[serde(default = "default_object")]
    pub object: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub outputs: Option<Vec<NativeObject>>,
    pub steps: Option<Vec<NativeObject>>,
    pub usage: Option<NativeObject>,
    #[serde(flatten, skip_deserializing)]
    pub extension: NativeObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct StreamingRecord {
    pub event_type: Option<String>,
    pub id: Option<String>,
    #[serde(default = "default_object")]
    pub object: Option<String>,
    pub model: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
    pub created: Option<String>,
    pub updated: Option<String>,
    pub outputs: Option<Vec<NativeObject>>,
    pub steps: Option<Vec<NativeObject>>,
    pub usage: Option<NativeObject>,
    pub delta: Option<NativeObject>,
    pub index: Option<i64>,
    pub step: Option<NativeObject>,
    pub interaction_id: Option<String>,
    pub interaction: Option<NativeObject>,
    #[serde(flatten, skip_deserializing)]
    pub extension: NativeObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct CancelInteractionResult {
    pub id: Option<String>,
    pub status: Option<String>,
    #[serde(flatten, skip_deserializing)]
    pub extension: NativeObject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeleteInteractionResult {
    pub success: bool,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenParams {
    pub headers: IndexMap<String, String>,
    pub additional_headers: Option<AdditionalHeaders>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InteractionResponse {
    pub record: InteractionRecord,
    pub hidden_params: HiddenParams,
}

impl NativeRecord for InteractionRecord {
    const KNOWN_FIELDS: &'static [&'static str] = &[
        "id", "object", "model", "agent", "status", "created", "updated", "outputs", "steps",
        "usage",
    ];

    fn set_extension(&mut self, extension: NativeObject) {
        self.extension = extension;
    }
}

impl NativeRecord for StreamingRecord {
    const KNOWN_FIELDS: &'static [&'static str] = &[
        "event_type",
        "id",
        "object",
        "model",
        "agent",
        "status",
        "created",
        "updated",
        "outputs",
        "steps",
        "usage",
        "delta",
        "index",
        "step",
        "interaction_id",
        "interaction",
    ];

    fn set_extension(&mut self, extension: NativeObject) {
        self.extension = extension;
    }
}

impl NativeRecord for CancelInteractionResult {
    const KNOWN_FIELDS: &'static [&'static str] = &["id", "status"];

    fn set_extension(&mut self, extension: NativeObject) {
        self.extension = extension;
    }
}
