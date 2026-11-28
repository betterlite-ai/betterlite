use std::collections::BTreeMap;

use betterlite_bedrock::aws_settings::sources::Credentials;
use betterlite_bedrock::embedding_call::ParamMap;
use betterlite_bedrock::model_routes::ParamMap as ModelCost;
use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_provider::bedrock_other_dispatch::test_support::client::Reply;
use secrecy::SecretString;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::value::RawValue;
use serde_json::{Map, Value};

use crate::openai_media_dispatch::failure;

pub(super) type Failed = Box<HelperError>;

// The operation a probe runs through the one composition dispatcher
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Case {
    Embedding,
    EmbeddingStatus,
    Rerank,
    Count,
    Transcription,
    ImageGeneration,
    ImageEdit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Input {
    #[serde(default)]
    pub(super) model_cost: ModelCost,
    pub(super) probes: Vec<Probe>,
}

// One row's call: the fields a family never reads stay at their defaults
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Probe {
    pub(super) case: Case,
    pub(super) model: String,
    #[serde(default)]
    pub(super) input: Vec<String>,
    #[serde(default)]
    pub(super) prompt: Option<String>,
    #[serde(default)]
    pub(super) images: Vec<String>,
    #[serde(default)]
    pub(super) optional_params: BTreeMap<String, Value>,
    #[serde(default)]
    pub(super) query: Option<String>,
    #[serde(default)]
    pub(super) documents: Vec<RerankDocument>,
    #[serde(default)]
    pub(super) top_n: Option<i64>,
    #[serde(default)]
    pub(super) return_documents: Option<bool>,
    #[serde(default)]
    pub(super) rank_fields: Option<Vec<String>>,
    #[serde(default)]
    pub(super) messages: Option<Json>,
    #[serde(default)]
    pub(super) request_model: Option<String>,
    #[serde(default)]
    pub(super) invocation_arn: Option<String>,
    #[serde(default)]
    pub(super) region: Option<String>,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    #[serde(default)]
    pub(super) file: Option<File>,
    #[serde(default)]
    pub(super) timeout: Option<f64>,
    #[serde(default)]
    pub(super) timeout_ms: Option<u64>,
    #[serde(default)]
    pub(super) reply_text: Option<String>,
    #[serde(default)]
    pub(super) asynchronous: bool,
    #[serde(default)]
    pub(super) api_base: Option<String>,
    #[serde(default)]
    pub(super) api_key: Option<String>,
    #[serde(default)]
    pub(super) extra_headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) reply: ProbeReply,
    #[serde(default)]
    pub(super) replies: Vec<ProbeReply>,
}

// A typed file the transcription probe hands its leaf; the leaf names the format from the filename
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct File {
    pub(super) content: String,
    #[serde(default)]
    pub(super) filename: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ProbeReply {
    pub(super) status: u16,
    pub(super) headers: BTreeMap<String, String>,
    pub(super) body: String,
    pub(super) failure: bool,
}

impl Default for ProbeReply {
    fn default() -> Self {
        Self {
            status: 200,
            headers: BTreeMap::new(),
            body: String::from("{}"),
            failure: false,
        }
    }
}

impl ProbeReply {
    pub(super) fn answer(&self) -> Reply {
        if self.failure {
            return Reply::Failed;
        }
        if self.headers.is_empty() {
            return Reply::status(self.status, &self.body);
        }
        let pairs: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        Reply::with_headers(self.status, &pairs, &self.body)
    }
}

// The probe's own aws keys are the identity its credential source resolves
pub(super) fn credentials(probe: &Probe) -> Result<Credentials, Failed> {
    Ok(Credentials {
        access_key_id: SecretString::from(text_param(probe, "aws_access_key_id")?),
        secret_access_key: SecretString::from(text_param(probe, "aws_secret_access_key")?),
        session_token: None,
    })
}

fn text_param(probe: &Probe, name: &'static str) -> Result<String, Failed> {
    match probe.optional_params.get(name) {
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_other) => Err(failure("Credentials", format!("{name} is not a string"))),
        None => Err(failure("Credentials", format!("{name} is missing"))),
    }
}

pub(super) fn param_map(probe: &Probe) -> Result<ParamMap, Failed> {
    let mut map = ParamMap::new();
    for (name, value) in &probe.optional_params {
        let text = serde_json::to_string(value).map_err(|error| failure("Params", error))?;
        let raw = RawValue::from_string(text).map_err(|error| failure("Params", error))?;
        map.insert(name.clone(), raw);
    }
    Ok(map)
}

pub(super) fn from_params<T>(probe: &Probe) -> Result<T, Failed>
where
    T: DeserializeOwned,
{
    let object: Map<String, Value> = probe
        .optional_params
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    serde_json::from_value(Value::Object(object)).map_err(|error| failure("Params", error))
}

pub(super) fn extra_pairs(probe: &Probe) -> Option<Vec<(String, String)>> {
    probe.extra_headers.as_ref().map(|headers| {
        headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()
    })
}

pub(super) fn replies(probe: &Probe) -> Vec<Reply> {
    if probe.replies.is_empty() {
        return vec![probe.reply.answer()];
    }
    probe.replies.iter().map(ProbeReply::answer).collect()
}
