use std::sync::PoisonError;

use betterlite_azure::azure_stream_errors::error::{AzureStreamFailure, Reported, StreamingError};
use betterlite_azure::azure_stream_errors::records::PreCallRecord;
use betterlite_azure::azure_stream_errors::stream::{AzureChatStream, WrapperHeaders};
use betterlite_models::decisions::{Extra, Json};
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_streaming::stream_chunk_shaping::records::{StreamChunk, StreamOptions};
use secrecy::ExposeSecret as _;
use serde_json::Number;

use super::hooks::Recorder;
use super::source::credentials::ScriptedSources;
use super::source::{FormView, GetView, ScriptedSource};

mod outcomes;

pub(crate) use outcomes::{Outcome, error_report, report};

const CREATED_FLOOR: i64 = 1_000_000_000;

pub(super) fn calls(values: Vec<Json>) -> Json {
    let mut fields = Extra::new();
    fields.insert("calls".to_owned(), Json::List(values));
    Json::Object(fields)
}

fn text_map<'fields>(fields: impl IntoIterator<Item = (&'fields String, &'fields String)>) -> Json {
    Json::Object(
        fields
            .into_iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}

fn get_value(get: &GetView) -> Json {
    let mut fields = Extra::new();
    fields.insert(
        "api_version".to_owned(),
        Json::Text(get.api_version.clone()),
    );
    fields.insert("api_base".to_owned(), Json::Text(get.api_base.clone()));
    fields.insert(
        "api_key".to_owned(),
        get.api_key
            .as_ref()
            .map_or(Json::Null, |key| Json::Text(key.clone())),
    );
    fields.insert("model".to_owned(), Json::Text(get.model.clone()));
    fields.insert("is_async".to_owned(), Json::Bool(get.is_async));
    fields.insert("litellm_params".to_owned(), get.litellm_params.clone());
    Json::Object(fields)
}

pub(super) fn gets(source: &ScriptedSource) -> Vec<Json> {
    source
        .gets
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .map(get_value)
        .collect()
}

fn form_value(form: &FormView) -> Json {
    let mut fields = Extra::new();
    fields.insert("url".to_owned(), Json::Text(form.url.clone()));
    fields.insert("form".to_owned(), text_map(&form.form));
    Json::Object(fields)
}

pub(super) fn exchanges(sources: &ScriptedSources) -> Vec<Json> {
    let forms = sources.forms.lock().unwrap_or_else(PoisonError::into_inner);
    let mut values = Vec::with_capacity(forms.len());
    for form in &*forms {
        values.push(form_value(form));
    }
    values
}

// The input, headers and target logging_obj.pre_call saw
pub(super) fn pre_value(record: &PreCallRecord) -> Json {
    let args = &record.additional_args;
    let mut fields = Extra::new();
    fields.insert("input".to_owned(), record.input.clone());
    fields.insert(
        "api_key".to_owned(),
        record
            .api_key
            .as_ref()
            .map_or(Json::Null, |key| Json::Text(key.expose_secret().to_owned())),
    );
    fields.insert(
        "headers".to_owned(),
        args.headers
            .as_ref()
            .map_or(Json::Null, |headers| Json::Object(headers.clone())),
    );
    fields.insert(
        "api_base".to_owned(),
        args.api_base
            .as_deref()
            .map_or(Json::Null, |base| Json::Text(base.to_owned())),
    );
    fields.insert(
        "acompletion".to_owned(),
        Json::Bool(matches!(
            args.passthrough.get("acompletion"),
            Some(Json::Bool(true))
        )),
    );
    fields.insert(
        "complete_input_dict".to_owned(),
        args.complete_input_dict
            .as_ref()
            .map_or(Json::Null, Json::from),
    );
    Json::Object(fields)
}

// logging_obj.model_call_details["response_headers"] as the streaming paths stored it
pub(super) fn response_headers(recorder: &Recorder) -> Json {
    recorder
        .response_headers
        .as_ref()
        .map_or(Json::Null, text_map)
}

fn wrapper_headers(headers: &WrapperHeaders) -> Json {
    match headers {
        WrapperHeaders::Processed(fields) => text_map(fields),
        WrapperHeaders::Raw(fields) => text_map(fields.iter()),
    }
}

fn stream_options_value(options: Option<&StreamOptions>) -> Json {
    options.map_or(Json::Null, |options| {
        let mut fields = Extra::new();
        fields.insert(
            "include_usage".to_owned(),
            Json::Bool(options.include_usage),
        );
        Json::Object(fields)
    })
}

pub(super) fn wrapper_value(stream: &AzureChatStream) -> Json {
    let mut fields = Extra::new();
    fields.insert("model".to_owned(), Json::Text(stream.model.clone()));
    fields.insert(
        "provider".to_owned(),
        Json::Text(stream.custom_llm_provider.to_owned()),
    );
    fields.insert(
        "stream_options".to_owned(),
        stream_options_value(stream.stream_options.as_ref()),
    );
    fields.insert(
        "response_headers".to_owned(),
        wrapper_headers(&stream.response_headers),
    );
    fields.insert(
        "additional_headers".to_owned(),
        Json::Object(stream.additional_headers.clone()),
    );
    Json::Object(fields)
}

pub(super) fn chunk_value(chunk: &StreamChunk) -> Json {
    let choice = chunk.choices.first();
    let mut fields = Extra::new();
    fields.insert(
        "content".to_owned(),
        choice
            .and_then(|choice| choice.delta.content.clone())
            .map_or(Json::Null, Json::Text),
    );
    fields.insert(
        "role".to_owned(),
        choice
            .and_then(|choice| choice.delta.role.clone())
            .map_or(Json::Null, Json::Text),
    );
    fields.insert(
        "finish_reason".to_owned(),
        choice
            .and_then(|choice| choice.finish_reason.clone())
            .map_or(Json::Null, Json::Text),
    );
    fields.insert(
        "index".to_owned(),
        choice.map_or(Json::Null, |choice| {
            Json::Number(Number::from(choice.index))
        }),
    );
    fields.insert("model".to_owned(), Json::Text(chunk.model.clone()));
    fields.insert(
        "created_is_current".to_owned(),
        Json::Bool(chunk.created > CREATED_FLOOR),
    );
    Json::Object(fields)
}

pub(super) fn error_value(failure: &AzureStreamFailure) -> Json {
    let (status_present, status) = match &failure.status {
        Reported::Present(Some(value)) => (true, Json::Number(Number::from(*value))),
        Reported::Present(None) => (true, Json::Null),
        Reported::Absent => (false, Json::Null),
    };
    let (message_present, message) = match &failure.message {
        Reported::Present(Some(text)) => (true, Json::Text(text.clone())),
        Reported::Present(None) => (true, Json::Null),
        Reported::Absent => (false, Json::Null),
    };
    let mut fields = Extra::new();
    fields.insert("class".to_owned(), Json::Text(failure.class.clone()));
    fields.insert("status_present".to_owned(), Json::Bool(status_present));
    fields.insert("status".to_owned(), status);
    fields.insert("message_present".to_owned(), Json::Bool(message_present));
    fields.insert("message".to_owned(), message);
    fields.insert(
        "headers".to_owned(),
        failure
            .headers
            .as_ref()
            .map_or(Json::Null, |headers| text_map(&headers.0)),
    );
    fields.insert(
        "body".to_owned(),
        failure.body.clone().unwrap_or(Json::Null),
    );
    Json::Object(fields)
}

pub(super) fn failure(error: &StreamingError) -> Box<HelperError> {
    Box::new(HelperError {
        class: "StreamingError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
