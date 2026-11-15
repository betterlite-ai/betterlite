use betterlite_config::tls::Verification;
use betterlite_core::api::realtime_beta_protocol::headers::{
    HeaderField, HeaderPair, HeaderScope, openai_beta_realtime_header,
};
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_models::decisions::Json;
use betterlite_openai::realtime::connect::RealtimeConnectSettings;
use betterlite_openai::realtime::logging::{RealtimeLogRecord, RealtimeLogger};
use betterlite_openai::xai_realtime::connect::prepare_connection;
use betterlite_openai::xai_realtime::normalizer::{XaiRealtimeNormalizer, should_drop};
use betterlite_openai::xai_realtime::session::patch_outgoing_session;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::json;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

pub(crate) struct Op;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Sequence,
    Connection,
}

#[derive(Deserialize)]
pub(crate) struct Input {
    kind: Kind,
    events: Vec<Json>,
    sessions: Vec<Json>,
    model: String,
    api_key: Option<String>,
    scope_headers: Vec<(String, String)>,
    max_message_size_bytes: usize,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input.kind {
            Kind::Sequence => sequence_answer(input),
            Kind::Connection => connection_answer(input),
        }
    }
}

fn helper_error(message: &str) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("XaiRealtime"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    })
}

fn sequence_answer(input: &Input) -> HelperResult {
    let mut normalizer = XaiRealtimeNormalizer::default();
    let emitted = replay(&mut normalizer, &input.events)?;
    let sessions: Vec<Json> = input
        .sessions
        .iter()
        .cloned()
        .map(patch_outgoing_session)
        .collect();
    Ok(json!({"emitted": emitted, "sessions": sessions}))
}

fn replay(
    normalizer: &mut XaiRealtimeNormalizer,
    events: &[Json],
) -> Result<Vec<Json>, Box<HelperError>> {
    let mut emitted = Vec::with_capacity(events.len());
    for event in events {
        if should_drop(event) {
            continue;
        }
        let normalized = normalizer
            .normalize(event.clone())
            .map_err(|error| helper_error(&error.to_string()))?;
        emitted.push(normalized);
    }
    Ok(emitted)
}

struct RecordingLogger;

impl RealtimeLogger for RecordingLogger {
    fn pre_call(&mut self, _record: RealtimeLogRecord<'_>) -> Result<(), PrePostError> {
        Ok(())
    }
}

fn beta_realtime(input: &Input) -> bool {
    let headers: Vec<HeaderPair> = input
        .scope_headers
        .iter()
        .map(|(name, value)| {
            (
                HeaderField::Text(name.clone()),
                HeaderField::Text(value.clone()),
            )
        })
        .collect();
    openai_beta_realtime_header(Some(&HeaderScope { headers: &headers }))
}

fn connection_answer(input: &Input) -> HelperResult {
    let api_key = input.api_key.as_deref().map(SecretString::from);
    let verification = Verification::Enabled;
    let settings = RealtimeConnectSettings {
        api_base: None,
        api_key: api_key.as_ref(),
        model: &input.model,
        query_params: None,
        beta_realtime: beta_realtime(input),
        ssl_verification: &verification,
        max_message_size_bytes: input.max_message_size_bytes,
    };
    let mut logger = RecordingLogger;
    let connection = prepare_connection(&settings, &mut logger)
        .map_err(|error| helper_error(&error.to_string()))?;
    Ok(json!({
        "url": connection.url,
        "headers": connection.headers.iter().map(|(name, value)| (name.clone(), value.clone())).collect::<Vec<_>>(),
        "max_size": connection.max_message_size_bytes,
        "ssl": connection.ssl,
    }))
}
