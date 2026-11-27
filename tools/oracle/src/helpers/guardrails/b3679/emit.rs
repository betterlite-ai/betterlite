use std::collections::BTreeMap;
use std::future::{Future, ready};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_message_scope::StreamItem;
use betterlite_guardrails::guardrail_message_scope::stream_chunk_events::stream_item_value;
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};
use betterlite_guardrails::unified_dispatch::emit::{
    HttpErrorDetail, StreamErrorEmission, StreamHttpError, emit_streaming_http_error,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::support::{error_json, to_json};
use betterlite_oracle_helper_api::helper::HelperResult;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum DetailInput {
    Text(String),
    Object(BTreeMap<String, GuardrailValue>),
}

impl DetailInput {
    pub(super) fn into_detail(self) -> HttpErrorDetail {
        match self {
            Self::Text(text) => HttpErrorDetail::Text(text),
            Self::Object(fields) => HttpErrorDetail::Object(fields),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TranslationScript {
    pub(super) error_items: Option<Vec<GuardrailValue>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmissionRequest {
    pub(super) call_type: Option<CallType>,
    pub(super) status_code: u16,
    pub(super) detail: DetailInput,
    #[serde(default)]
    pub(super) responses_so_far: Vec<GuardrailValue>,
    #[serde(default)]
    pub(super) request_data: BTreeMap<String, GuardrailValue>,
    #[serde(default)]
    pub(super) stream_started: bool,
    pub(super) responses_yielded: Option<Vec<GuardrailValue>>,
    pub(super) translation: Option<TranslationScript>,
}

pub(super) struct ScriptedTranslation {
    error_items: Option<Vec<StreamItem>>,
}

impl BaseTranslation for ScriptedTranslation {
    type Guardrail = ();
    type LogTarget = ();
    type Failure = GuardrailFailure;

    fn process_input_messages(
        &self,
        _data: &mut BTreeMap<String, GuardrailValue>,
        _guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> impl Future<Output = Result<(), Self::Failure>> + Send {
        ready(Ok(()))
    }

    fn process_output_response(
        &self,
        response: GuardrailValue,
        _guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> impl Future<Output = Result<GuardrailValue, Self::Failure>> + Send {
        ready(Ok(response))
    }

    fn build_stream_error_items(
        &self,
        _failure: &Self::Failure,
        responses_so_far: Option<&[StreamItem]>,
    ) -> Option<Vec<StreamItem>> {
        let items = self.error_items.as_ref()?;
        let count = responses_so_far.map_or(0, <[StreamItem]>::len);
        let mut echoed = items.clone();
        echoed.push(StreamItem::Json(GuardrailValue::Text(format!(
            "items={count}"
        ))));
        Some(echoed)
    }
}

pub(super) fn emission(request: &EmissionRequest) -> HelperResult {
    let error = StreamHttpError {
        status_code: request.status_code,
        detail: request.detail.clone().into_detail(),
    };
    let scripted = request
        .translation
        .as_ref()
        .map(|script| ScriptedTranslation {
            error_items: script
                .error_items
                .as_ref()
                .map(|items| items.iter().cloned().map(StreamItem::Json).collect()),
        });
    let responses_so_far: Vec<StreamItem> = request
        .responses_so_far
        .iter()
        .cloned()
        .map(StreamItem::Json)
        .collect();
    let responses_yielded = request.responses_yielded.as_ref().map(|items| {
        items
            .iter()
            .cloned()
            .map(StreamItem::Json)
            .collect::<Vec<_>>()
    });
    let outcome = emit_streaming_http_error(
        error,
        &StreamErrorEmission {
            call_type: request.call_type,
            responses_so_far: &responses_so_far,
            request_data: &request.request_data,
            translation: scripted.as_ref(),
            stream_started: request.stream_started,
            responses_yielded: responses_yielded.as_deref(),
        },
    );
    match outcome {
        Ok(items) => {
            let collected: Result<Vec<Value>, _> = items
                .iter()
                .map(|item| to_json(&stream_item_value(item)))
                .collect();
            Ok(json!({"items": collected?}))
        }
        Err(failure) => Ok(json!({"error": error_json(&failure)?})),
    }
}
