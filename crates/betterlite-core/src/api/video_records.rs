use betterlite_models::decisions::Json;
use betterlite_openai::video_edits::response::{CharacterKind, VideoKind};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::bfl::edit::media::SuppliedMedia;

pub mod dict_access;
pub mod helpers;

// The pydantic model the video routes return: json() carries the declared fields, and the private
// _hidden_params answers only contains, get and getitem.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoObject {
    pub id: String,
    pub object: VideoKind,
    pub status: String,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub completed_at: Option<i64>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub error: Option<IndexMap<String, Json>>,
    #[serde(default)]
    pub progress: Option<i64>,
    #[serde(default)]
    pub remixed_from_video_id: Option<String>,
    #[serde(default)]
    pub seconds: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub usage: Option<IndexMap<String, Json>>,
    #[serde(skip)]
    pub hidden_params: IndexMap<String, Json>,
}

impl Default for VideoObject {
    fn default() -> Self {
        Self {
            id: String::new(),
            object: VideoKind::Video,
            status: String::new(),
            created_at: None,
            completed_at: None,
            expires_at: None,
            error: None,
            progress: None,
            remixed_from_video_id: None,
            seconds: None,
            size: None,
            model: None,
            usage: None,
            hidden_params: IndexMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharacterObject {
    pub id: String,
    #[serde(default)]
    pub object: CharacterKind,
    pub created_at: i64,
    pub name: String,
    #[serde(skip)]
    pub hidden_params: IndexMap<String, Json>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecodedVideoId {
    #[serde(default)]
    pub custom_llm_provider: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    pub video_id: String,
}

// LiteLLM's VideoCreateOptionalRequestParams, a total=False TypedDict whose resolution key is ReadOnly: nothing is required and the type only names what a caller may send.
#[derive(Debug, Clone, PartialEq, Eq)]
// ast-grep-ignore: no-long-names
pub struct VideoCreateOptionalRequestParams {
    pub input_reference: Option<SuppliedMedia>,
    pub image: Option<Json>,
    pub parameters: Option<IndexMap<String, Json>>,
    pub model: Option<String>,
    pub resolution: Option<String>,
    pub seconds: Option<String>,
    pub size: Option<String>,
    pub characters: Option<Vec<IndexMap<String, String>>>,
    pub user: Option<String>,
    pub extra_headers: Option<IndexMap<String, String>>,
    pub extra_body: Option<IndexMap<String, String>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VideoRecordError {
    #[error("{object} has no field {field}")]
    UnknownField { object: &'static str, field: String },
    #[error("{field} must be a string")]
    NotText { field: &'static str },
}

// check names fix these test modules as api::video_records::<check>, so their declarations live under tests/
include!("video_records/tests/modules.rs");
