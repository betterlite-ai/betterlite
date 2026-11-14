use std::collections::BTreeMap;
use std::mem;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use betterlite_core::routes::CallType;
use serde::Deserialize as _;
use serde::de::IntoDeserializer as _;
use serde::de::value::{Error as TextError, StrDeserializer};
use thiserror::Error;

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_selection::gate::GuardrailSelection as _;
use crate::guardrail_translation::{BaseTranslation, TranslationCall};
use crate::unified_dispatch::registry::{TranslationEntry, TranslationRegistry};

type Data = BTreeMap<String, GuardrailValue>;

const GUARDRAIL_INFORMATION_KEY: &str = "standard_logging_guardrail_information";

// Whether the logged result is a ModelResponse, which picks the acompletion translation for the output
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoggedResultKind {
    ModelResponse,
    Other,
}

// What the logging layer hands the hook: the logged kwargs, the call's result and the call type name
#[derive(Debug, Clone)]
pub struct LoggedCall<'call> {
    pub kwargs: Data,
    pub result: &'call GuardrailValue,
    pub result_kind: LoggedResultKind,
    pub call_type: &'call str,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoggingHookError {
    #[error("{field} must be an object")]
    NotObject { field: &'static str },
    #[error("standard_logging_object.guardrail_information must be a list")]
    ExistingNotList,
    #[error("no acompletion translation is registered for a ModelResponse")]
    NoOutputTranslation,
}

// What went wrong inside the scan; the hook logs it and carries on
#[derive(Debug, Error)]
enum ScanFailure<F> {
    #[error("{0}")]
    Request(LoggingHookError),
    #[error("{0}")]
    Translation(F),
}

struct LoggedScan<'scan, T: BaseTranslation> {
    input: &'scan T,
    output: &'scan T,
    guardrail: &'scan T::Guardrail,
    response: Option<GuardrailValue>,
}

// A ModelResponse is always scanned by the acompletion translation
fn output_translation<'registry, T>(
    registry: &'registry TranslationRegistry<T>,
    input: &'registry TranslationEntry<T>,
    model_response: bool,
) -> Result<&'registry T, LoggingHookError> {
    if !model_response {
        return Ok(&input.translation);
    }
    registry
        .get(CallType::Acompletion)
        .map(|entry| &entry.translation)
        .ok_or(LoggingHookError::NoOutputTranslation)
}

fn parse_call_type(name: &str) -> Option<CallType> {
    let text: StrDeserializer<'_, TextError> = name.into_deserializer();
    CallType::deserialize(text).ok()
}

// The response LiteLLM scans: the assembled streaming response when there is one, else the call's result
fn logged_response(kwargs: &Data, result: &GuardrailValue) -> (Option<GuardrailValue>, bool) {
    for key in [
        "async_complete_streaming_response",
        "complete_streaming_response",
    ] {
        if let Some(found) = kwargs.get(key)
            && *found != GuardrailValue::Null
        {
            return (Some(found.clone()), true);
        }
    }
    let result_found = (*result != GuardrailValue::Null).then(|| result.clone());
    (result_found, false)
}

// The logged request's metadata without the verdict record, so the scan writes a fresh one
fn scratch_metadata(kwargs: &Data) -> Result<Data, LoggingHookError> {
    let metadata = match kwargs.get("litellm_params") {
        None | Some(GuardrailValue::Null) => return Ok(Data::new()),
        Some(GuardrailValue::Object(params)) => params.get("metadata"),
        Some(_) => {
            return Err(LoggingHookError::NotObject {
                field: "litellm_params",
            });
        }
    };
    match metadata {
        None | Some(GuardrailValue::Null) => Ok(Data::new()),
        Some(GuardrailValue::Object(fields)) => Ok(fields
            .iter()
            .filter(|(name, _value)| name.as_str() != GUARDRAIL_INFORMATION_KEY)
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect()),
        Some(_) => Err(LoggingHookError::NotObject {
            field: "litellm_params.metadata",
        }),
    }
}

fn logged_tools(kwargs: &Data) -> Result<GuardrailValue, LoggingHookError> {
    match kwargs.get("optional_params") {
        None | Some(GuardrailValue::Null) => Ok(GuardrailValue::Null),
        Some(GuardrailValue::Object(params)) => {
            Ok(params.get("tools").cloned().unwrap_or(GuardrailValue::Null))
        }
        Some(_) => Err(LoggingHookError::NotObject {
            field: "optional_params",
        }),
    }
}

// The copy of the logged request the scan works on; messages and input both hold the logged messages, else the logged input
fn scratch_request(kwargs: &Data, tools: GuardrailValue, metadata: Data) -> Data {
    let logged = |field: &str| kwargs.get(field).cloned().unwrap_or(GuardrailValue::Null);
    let logged_input = match kwargs.get("messages") {
        Some(messages) if *messages != GuardrailValue::Null => messages.clone(),
        _ => logged("input"),
    };
    BTreeMap::from([
        ("model".to_owned(), logged("model")),
        ("messages".to_owned(), logged_input.clone()),
        ("input".to_owned(), logged_input),
        ("tools".to_owned(), tools),
        ("litellm_call_id".to_owned(), logged("litellm_call_id")),
        ("metadata".to_owned(), GuardrailValue::Object(metadata)),
    ])
}

async fn run_scan<T: BaseTranslation>(
    scan: LoggedScan<'_, T>,
    request: &mut Data,
) -> Result<(), T::Failure> {
    scan.input
        .process_input_messages(request, scan.guardrail, None)
        .await?;
    if let Some(response) = scan.response {
        let call = TranslationCall {
            user_api_key_dict: None,
            request_data: Some(request),
            logging_obj: None,
            stream_transform_sink: None,
            deliver_ended_stream_rewrites: false,
        };
        scan.output
            .process_output_response(response, scan.guardrail, call)
            .await?;
    }
    Ok(())
}

// LiteLLM's _scan_logged_call; whatever the scan recorded is left in metadata, also when it fails part way
async fn scan_logged_call<T: BaseTranslation>(
    scan: LoggedScan<'_, T>,
    kwargs: &Data,
    metadata: &mut Data,
) -> Result<(), ScanFailure<T::Failure>> {
    let tools = logged_tools(kwargs).map_err(ScanFailure::Request)?;
    let mut request = scratch_request(kwargs, tools, mem::take(metadata));
    let scanned = run_scan(scan, &mut request).await;
    if let Some(GuardrailValue::Object(returned)) = request.remove("metadata") {
        *metadata = returned;
    }
    scanned.map_err(ScanFailure::Translation)
}

// Adds the recorded verdict to standard_logging_object.guardrail_information; kwargs stays as it was when there is nothing to add
fn with_recorded(
    mut kwargs: Data,
    recorded: Option<GuardrailValue>,
) -> Result<Data, LoggingHookError> {
    let entries = match recorded {
        None | Some(GuardrailValue::Null) => return Ok(kwargs),
        Some(GuardrailValue::Array(list)) if list.is_empty() => return Ok(kwargs),
        Some(GuardrailValue::Array(list)) => list,
        Some(single) => vec![single],
    };
    let Some(GuardrailValue::Object(logged)) = kwargs.get("standard_logging_object") else {
        return Ok(kwargs);
    };
    let mut information = match logged.get("guardrail_information") {
        None | Some(GuardrailValue::Null) => Vec::new(),
        Some(GuardrailValue::Array(existing)) => existing.clone(),
        Some(_) => return Err(LoggingHookError::ExistingNotList),
    };
    information.extend(entries);
    let mut updated = logged.clone();
    updated.insert(
        "guardrail_information".to_owned(),
        GuardrailValue::Array(information),
    );
    kwargs.insert(
        "standard_logging_object".to_owned(),
        GuardrailValue::Object(updated),
    );
    Ok(kwargs)
}

impl GuardrailBase {
    // LiteLLM's async_logging_hook: runs a logging_only guardrail on copies of the logged call and records its verdict
    pub async fn logging_hook<T: BaseTranslation>(
        &self,
        guardrail: &T::Guardrail,
        registry: &TranslationRegistry<T>,
        call: LoggedCall<'_>,
    ) -> Result<Data, LoggingHookError> {
        if !self.settings.switches.uses_apply_guardrail_interface
            || !self.event_hook_is_event_type(GuardrailEventHooks::LoggingOnly)
        {
            return Ok(call.kwargs);
        }
        let name = self.settings.guardrail_name.as_deref().unwrap_or("None");
        let Some(input) = parse_call_type(call.call_type).and_then(|kind| registry.get(kind))
        else {
            tracing::debug!(
                "Guardrail {name}: no guardrail translation for call_type={}, skipping logging_only scan",
                call.call_type
            );
            return Ok(call.kwargs);
        };
        let mut metadata = scratch_metadata(&call.kwargs)?;
        let (response, from_stream) = logged_response(&call.kwargs, call.result);
        let model_response = from_stream || call.result_kind == LoggedResultKind::ModelResponse;
        let scan = LoggedScan {
            input: &input.translation,
            output: output_translation(registry, input, model_response)?,
            guardrail,
            response,
        };
        if let Err(error) = scan_logged_call(scan, &call.kwargs, &mut metadata).await {
            tracing::warn!("Guardrail {name}: logging_only scan raised: {error}");
        }
        with_recorded(call.kwargs, metadata.remove(GUARDRAIL_INFORMATION_KEY))
    }
}
