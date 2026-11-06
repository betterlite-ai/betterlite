use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::fields::payload::records::{AdditionalHeaders, StandardLoggingHiddenParams};

// The init_response_obj shapes extract distinguishes; a native response's type stays with its endpoint owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InitResponseObject {
    Absent,
    Mapping(IndexMap<String, Json>),
    Projected {
        body: IndexMap<String, Json>,
        hidden_params: Option<IndexMap<String, Json>>,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HiddenParams {
    FromObject(IndexMap<String, Json>),
    FromException(Box<StandardLoggingHiddenParams>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedResponse {
    pub response_obj: IndexMap<String, Json>,
    pub hidden_params: Option<HiddenParams>,
}

// The text the client actually received, one entry per choice; B252 records it before logging.
pub type ServedTexts = Vec<Option<String>>;

// The two injected steps get_final_response_obj runs: the served-text overlay B252 owns and the redaction B188 owns.
pub trait ResponsePrivacy {
    fn overlay(&self, response: Json, served_texts: Option<&ServedTexts>) -> Json;
    fn redact(&self, response: Json) -> Json;
}

// _extract_response_obj_and_hidden_params: a mapping logs as sent, a projected response supplies its dump and hidden params.
#[must_use]
pub fn extract(
    init: InitResponseObject,
    exception_headers: Option<AdditionalHeaders>,
) -> ExtractedResponse {
    let (response_obj, object_hidden) = match init {
        InitResponseObject::Absent | InitResponseObject::Other => (IndexMap::new(), None),
        InitResponseObject::Mapping(fields) => (fields, None),
        InitResponseObject::Projected {
            body,
            hidden_params,
        } => (body, hidden_params),
    };
    let hidden_params = match (object_hidden, exception_headers) {
        (Some(fields), _) => Some(HiddenParams::FromObject(fields)),
        (None, Some(headers)) => Some(HiddenParams::FromException(Box::new(
            StandardLoggingHiddenParams {
                model_id: None,
                cache_key: None,
                api_base: None,
                response_cost: None,
                litellm_overhead_time_ms: None,
                additional_headers: Some(headers),
                batch_models: None,
                batch_successful_requests: None,
                batch_failed_requests: None,
                litellm_model_name: None,
                usage_object: None,
            },
        ))),
        (None, None) => None,
    };
    ExtractedResponse {
        response_obj,
        hidden_params,
    }
}

// StandardLoggingPayloadSetup.get_final_response_obj: the response body wins, then a text or list init response, then {}.
#[must_use]
pub fn get_final_response_obj(
    response_obj: Option<&IndexMap<String, Json>>,
    init_response_obj: &Json,
    served_texts: Option<&ServedTexts>,
    privacy: &dyn ResponsePrivacy,
) -> Json {
    let empty_body = match init_response_obj {
        Json::List(_) | Json::Text(_) => init_response_obj.clone(),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Object(_) => {
            Json::Object(IndexMap::new())
        }
    };
    let final_response_obj = response_obj.map_or(empty_body, |fields| Json::Object(fields.clone()));
    let overlaid = privacy.overlay(final_response_obj, served_texts);
    privacy.redact(overlaid)
}

// _provider_response_id: a mapping's non-empty text id, and nothing for any other source shape.
#[must_use]
pub fn provider_response_id(source: &Json) -> Option<String> {
    let fields = match source {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            return None;
        }
    };
    match fields.get("id") {
        Some(Json::Text(id)) if !id.is_empty() => Some(id.clone()),
        Some(
            Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::Text(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => None,
    }
}

mod tests;
