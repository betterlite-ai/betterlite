use betterlite_config::sections::settings::JsonValue;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::context::LogContext;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::post_call::{PostCall, post_call};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::api::audio_file_input::{AudioFile, get_audio_file_name};
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

pub const REDACTED_VALUE: &str = "[REDACTED]";
pub const SECRET_FIELD: &str = "webhook_auth_header_value";
pub const TRANSCRIPTIONS_PATH: &str = "/v1/transcriptions";
pub const ATRANSCRIPTION_FIELD: &str = "atranscription";

pub struct SonioxPreCall {
    pub url: String,
    pub body: IndexMap<String, Json>,
}

pub struct SonioxPostCall {
    pub input: Json,
    pub original_response: Json,
    pub body: IndexMap<String, Json>,
}

pub enum SonioxEvent {
    PreCall(SonioxPreCall),
    PostCall(SonioxPostCall),
}

pub trait SonioxLogger {
    fn record(&mut self, event: SonioxEvent) -> Result<(), PrePostError>;
}

impl SonioxLogger for LogContext<'_, '_> {
    fn record(&mut self, event: SonioxEvent) -> Result<(), PrePostError> {
        match event {
            SonioxEvent::PreCall(record) => {
                let call = PreCall {
                    input: Json::Null,
                    model: None,
                    additional_args: AdditionalArgs {
                        api_base: Some(record.url),
                        headers: None,
                        complete_input_dict: Some(RawBody::Object(record.body)),
                        request_str: None,
                        openai_sdk: None,
                        passthrough: IndexMap::from([(
                            String::from(ATRANSCRIPTION_FIELD),
                            Json::Bool(true),
                        )]),
                    },
                };
                pre_call(self.state, &call, self.env).map(drop)
            }
            SonioxEvent::PostCall(record) => {
                let call = PostCall {
                    original_response: record.original_response,
                    input: record.input,
                    additional_args: AdditionalArgs {
                        api_base: None,
                        headers: None,
                        complete_input_dict: Some(RawBody::Object(record.body)),
                        request_str: None,
                        openai_sdk: None,
                        passthrough: IndexMap::new(),
                    },
                };
                post_call(self.state, &call, self.env);
                Ok(())
            }
        }
    }
}

// Callbacks are best-effort: a failing observability integration never breaks the Soniox call.
pub fn log_pre_call(logger: &mut dyn SonioxLogger, api_base: &str, body: &Object) {
    let record = SonioxPreCall {
        url: format!("{api_base}{TRANSCRIPTIONS_PATH}"),
        body: redacted_body(body),
    };
    drop(logger.record(SonioxEvent::PreCall(record)));
}

pub fn log_post_call(
    logger: &mut dyn SonioxLogger,
    audio: Option<&AudioFile<'_>>,
    original_response: &Object,
    body: &Object,
) {
    let input = audio
        .filter(|file| carries_audio(file))
        .map_or(Json::Null, |file| Json::Text(get_audio_file_name(file)));
    let record = SonioxPostCall {
        input,
        original_response: Json::Object(logged_object(original_response)),
        body: redacted_body(body),
    };
    drop(logger.record(SonioxEvent::PostCall(record)));
}

#[must_use]
pub fn redacted_body(body: &Object) -> IndexMap<String, Json> {
    let mut redacted = logged_object(body);
    if let Some(value) = redacted.get_mut(SECRET_FIELD)
        && !matches!(value, Json::Null)
    {
        *value = Json::Text(String::from(REDACTED_VALUE));
    }
    redacted
}

fn logged_value(value: &JsonValue) -> Json {
    match value {
        JsonValue::Null => Json::Null,
        JsonValue::Bool(flag) => Json::Bool(*flag),
        JsonValue::Number(number) => Json::Number(number.clone()),
        JsonValue::String(text) => Json::Text(text.clone()),
        JsonValue::Array(items) => Json::List(items.iter().map(logged_value).collect()),
        JsonValue::Object(fields) => Json::Object(logged_object(fields)),
    }
}

fn logged_object(fields: &Object) -> IndexMap<String, Json> {
    fields
        .iter()
        .map(|(name, value)| (name.clone(), logged_value(value)))
        .collect()
}

// A falsy audio input (empty bytes, an empty string) logs no filename, as in LiteLLM.
const fn carries_audio(file: &AudioFile<'_>) -> bool {
    match file {
        AudioFile::Bytes(content) | AudioFile::ByteArray(content) => !content.is_empty(),
        AudioFile::Text(text) => !text.is_empty(),
        AudioFile::Path(_)
        | AudioFile::Stream(_)
        | AudioFile::Tuple(_)
        | AudioFile::Unsupported { .. } => true,
    }
}
