use betterlite_models::decisions::Json;
use chrono::{DateTime, FixedOffset};
use serde_json::Number;

use super::CallWrapper;
use super::error::CallError;
use super::fields::field;
use super::max_tokens::MODIFIED_CALL_TYPES;
use super::metadata::MetadataUpdate;
use super::ports::{CallTimes, WrapperClock};
use super::state::LoggingHandle;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;
use crate::call_setup::credentials::load_credentials_from_list;
use crate::call_setup::text::debug_text;

// The request and the model the decorator read from it, as update_response_metadata and the failure log want them
pub(super) struct CallView<'view> {
    pub(super) request: &'view RequestMap,
    pub(super) model: Option<&'view str>,
}

pub(super) fn insert_call_id<E>(wrapper: &CallWrapper<E>, request: &mut RequestMap) {
    if request.contains_key("litellm_call_id") {
        return;
    }
    let call_id = wrapper.ids.next_call_id();
    request.insert("litellm_call_id".to_owned(), Json::Text(call_id));
}

// print_args_passed_to_litellm: it prints and does nothing else
pub(super) fn debug_request(call_type: &str, request: &RequestMap, debugging: bool) {
    match debug_text(call_type, request, debugging) {
        Ok(Some(text)) => tracing::debug!("{text}"),
        Ok(None) => {}
        Err(error) => tracing::debug!(%error, "the debug request line could not be rendered"),
    }
}

pub(super) fn load_credentials<E>(
    wrapper: &CallWrapper<E>,
    request: &mut RequestMap,
) -> Result<(), CallError> {
    let warning = load_credentials_from_list(request, wrapper.credentials.as_ref())?;
    if let Some(found) = warning {
        tracing::debug!(
            credential = %found.credential_name,
            "no credential matches the requested name"
        );
    }
    Ok(())
}

pub(super) fn modify_max_tokens<E>(
    wrapper: &CallWrapper<E>,
    call_type: &str,
    request: &mut RequestMap,
    model: Option<&str>,
) -> Result<(), CallError> {
    let user_max_tokens: Option<i64> = field(request, "max_tokens")?;
    let Some(user_tokens) = user_max_tokens else {
        return Ok(());
    };
    let Some(model_name) = model else {
        return Ok(());
    };
    if !wrapper.flags.modify_params || !MODIFIED_CALL_TYPES.contains(&call_type) {
        return Ok(());
    }
    let hf_model_name: Option<String> = field(request, "hf_model_name")?;
    let messages = request.get("messages");
    let modified = wrapper.max_tokens.modified_max_tokens(
        model_name,
        hf_model_name.as_deref(),
        messages,
        user_tokens,
    );
    let value = modified.map_or(Json::Null, |tokens| Json::Number(Number::from(tokens)));
    request.insert("max_tokens".to_owned(), value);
    Ok(())
}

// stream_chunk_builder over the drained stream; None is the builder's own answer for no chunks
pub(super) fn assemble<E>(
    wrapper: &CallWrapper<E>,
    request: &RequestMap,
    logging: Option<&LoggingHandle>,
    outcome: &CallOutcome,
) -> Result<CallOutcome, CallError> {
    let messages = request.get("messages");
    let assembled = wrapper
        .stream_assembly
        .assemble(outcome, messages, logging)?;
    Ok(CallOutcome::Response(assembled.unwrap_or(Json::Null)))
}

pub(super) fn update_metadata<E>(
    wrapper: &CallWrapper<E>,
    logging: Option<&LoggingHandle>,
    call: &CallView<'_>,
    times: CallTimes,
    outcome: &CallOutcome,
) {
    let Some(handle) = logging else {
        return;
    };
    wrapper.metadata.update(
        handle,
        MetadataUpdate {
            outcome,
            model: call.model,
            request: call.request,
            times,
        },
    );
}

pub(super) fn times(
    start: DateTime<FixedOffset>,
    end: Option<DateTime<FixedOffset>>,
    clock: &dyn WrapperClock,
) -> CallTimes {
    CallTimes {
        start,
        end: end.unwrap_or_else(|| clock.now()),
    }
}
