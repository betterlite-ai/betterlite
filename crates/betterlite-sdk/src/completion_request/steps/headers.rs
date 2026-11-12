use super::convert::typed_from;
use super::messages::optional;
use crate::completion_options::headers::merge_request_headers;
use crate::completion_request::Preparation;
use crate::completion_request::args::JsonObject;
use crate::completion_request::call::PreparedCall;
use crate::completion_request::error::RequestResult;
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::{
    AffinityParams, SessionMetadata, add_provider_affinity_header,
};
use betterlite_models::headers::provider_specific::{
    ProviderSpecificHeader, provider_specific_headers,
};
use indexmap::IndexMap;
pub(crate) fn merge(state: &mut Preparation<'_>) -> RequestResult<()> {
    let headers = optional::<JsonObject>(&state.kwargs, "headers")?;
    let proxy = &state.settings.proxy_headers;
    let extra = state.args.extra_headers.as_ref();
    state.call.headers = merge_request_headers(headers.as_ref(), extra, proxy);
    Ok(())
}
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum HeaderEntries {
    One(ProviderSpecificHeader),
    Many(Vec<ProviderSpecificHeader>),
}
fn provider_header_entries(
    state: &Preparation<'_>,
) -> RequestResult<Option<Vec<ProviderSpecificHeader>>> {
    let Some(value) = state
        .kwargs
        .get("provider_specific_header")
        .filter(|value| !matches!(value, Json::Null))
    else {
        return Ok(None);
    };
    match typed_from::<HeaderEntries>(value, "provider_specific_header")? {
        HeaderEntries::One(entry) => Ok(Some(vec![entry])),
        HeaderEntries::Many(entries) => Ok(Some(entries)),
    }
}
pub(crate) fn provider_specific(state: &mut Preparation<'_>) -> RequestResult<()> {
    let entries = provider_header_entries(state)?;
    let provider = state.call.custom_llm_provider.as_deref();
    let scoped = provider_specific_headers(entries.as_deref(), provider);
    apply_text_headers(&mut state.call, scoped);
    Ok(())
}
fn apply_text_headers(call: &mut PreparedCall, entries: IndexMap<String, String>) {
    for (name, value) in entries {
        call.headers.insert(name, Json::Text(value));
    }
}
fn text_headers(headers: &JsonObject) -> IndexMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| match value {
            Json::Text(text) => Some((name.clone(), text.clone())),
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => None,
        })
        .collect()
}
pub(crate) fn affinity(state: &mut Preparation<'_>) -> RequestResult<()> {
    let params = AffinityParams {
        provider_affinity_header: state.settings.optional.provider_affinity_header.clone(),
        session_id: optional::<String>(&state.kwargs, "session_id")?,
        litellm_session_id: optional::<String>(&state.kwargs, "litellm_session_id")?,
        metadata: optional::<SessionMetadata>(&state.kwargs, "metadata")?,
        litellm_metadata: optional::<SessionMetadata>(&state.kwargs, "litellm_metadata")?,
    };
    let updated = add_provider_affinity_header(&text_headers(&state.call.headers), &params)?;
    apply_text_headers(&mut state.call, updated);
    Ok(())
}
