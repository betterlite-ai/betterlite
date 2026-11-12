use super::Preparation;
use super::convert::typed_from;
use super::messages::optional;
use crate::completion_options::headers::merge_request_headers;
use crate::completion_request::args::JsonObject;
use crate::completion_request::error::{CompletionRequestError, RequestResult};
use betterlite_models::decisions::Json;
use betterlite_models::headers::affinity::{
    AffinityParams, SessionMetadata, add_provider_affinity_header,
};
use betterlite_models::headers::provider_specific::{
    ProviderSpecificHeader, provider_specific_headers,
};
use indexmap::IndexMap;
pub(crate) fn merge(state: &mut Preparation<'_>) -> RequestResult<()> {
    let headers = match state.kwargs.get("headers") {
        None | Some(Json::Null) => None,
        Some(Json::Object(entries)) => Some(entries.clone()),
        Some(_) => {
            return Err(CompletionRequestError::FieldType {
                field: "headers",
                expected: "an object",
            });
        }
    };
    state.call.headers = merge_request_headers(
        headers.as_ref(),
        state.args.extra_headers.as_ref(),
        &state.settings.proxy_headers,
    );
    Ok(())
}
fn provider_header_entries(
    state: &Preparation<'_>,
) -> RequestResult<Option<Vec<ProviderSpecificHeader>>> {
    match state.kwargs.get("provider_specific_header") {
        None | Some(Json::Null) => Ok(None),
        Some(value @ Json::Object(_)) => Ok(Some(vec![typed_from::<ProviderSpecificHeader>(
            value,
            "provider_specific_header",
        )?])),
        Some(Json::List(items)) => Ok(Some(
            items
                .iter()
                .map(|item| typed_from::<ProviderSpecificHeader>(item, "provider_specific_header"))
                .collect::<RequestResult<Vec<_>>>()?,
        )),
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_)) => {
            Err(CompletionRequestError::FieldType {
                field: "provider_specific_header",
                expected: "an object or a list of objects",
            })
        }
    }
}
pub(crate) fn provider_specific(state: &mut Preparation<'_>) -> RequestResult<()> {
    let entries = provider_header_entries(state)?;
    let scoped = provider_specific_headers(
        entries.as_deref(),
        state.call.custom_llm_provider.as_deref(),
    );
    for (name, value) in scoped {
        state.call.headers.insert(name, Json::Text(value));
    }
    Ok(())
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
    for (name, value) in updated {
        state.call.headers.insert(name, Json::Text(value));
    }
    Ok(())
}
