use betterlite_openai::realtime::url::QueryParams;

use super::error::RealtimeError;
use super::protocol::{TRANSCRIPTION_INTENT, uses_beta_protocol};
use crate::azure_urls::url_parts::encoded_query;

const GA_PATH: &str = "/openai/v1/realtime";
const BETA_PATH: &str = "/openai/realtime";
const MODEL_KEY: &str = "model";
pub(crate) const INTENT_KEY: &str = "intent";
const API_VERSION_KEY: &str = "api-version";
const DEPLOYMENT_KEY: &str = "deployment";

pub(crate) fn construct_url(
    api_base: &str,
    model: &str,
    api_version: Option<&str>,
    realtime_protocol: Option<&str>,
    query_params: Option<&QueryParams>,
) -> Result<String, RealtimeError> {
    let base = api_base.replace("https://", "wss://");
    let beta = uses_beta_protocol(realtime_protocol);
    let requested_intent = query_params
        .and_then(|params| params.get(INTENT_KEY))
        .and_then(Option::as_deref);
    let ga_model = !beta
        && requested_intent != Some(TRANSCRIPTION_INTENT)
        && query_params.is_none_or(|params| params.contains_key(MODEL_KEY));
    let mut pairs: Vec<(String, String)> = Vec::with_capacity(3);
    if beta {
        let version = api_version.ok_or(RealtimeError::MissingApiVersion)?;
        pairs.push((String::from(API_VERSION_KEY), version.to_owned()));
        pairs.push((String::from(DEPLOYMENT_KEY), model.to_owned()));
    }
    if ga_model {
        pairs.push((String::from(MODEL_KEY), model.to_owned()));
    }
    if let Some(value) = requested_intent.filter(|value| !value.is_empty()) {
        pairs.push((String::from(INTENT_KEY), value.to_owned()));
    }
    let path = if beta { BETA_PATH } else { GA_PATH };
    Ok(encoded_query(&pairs).map_or_else(
        || format!("{base}{path}"),
        |query| format!("{base}{path}?{query}"),
    ))
}
