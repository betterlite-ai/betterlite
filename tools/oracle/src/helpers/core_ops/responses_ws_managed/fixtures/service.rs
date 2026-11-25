use super::managed::{Op, input::Input};
use crate::websocket_driver::service::Arguments;
use betterlite_core::api::response_id_traversal::response_id::transcode;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper as _, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Deserialize)]
struct Observation {
    frames: Vec<Value>,
    #[serde(flatten)]
    effects: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct Warmup {
    response: WarmupResponse,
}

#[derive(Deserialize)]
struct WarmupResponse {
    id: String,
    created_at: i64,
}

fn failure(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("ManagedTranscriptError"),
        message,
        status: None,
        error_type: None,
        param: None,
        code: None,
    })
}

pub(crate) fn run(arguments: &Arguments) -> HelperResult {
    if arguments.managed.is_empty() {
        return Err(failure(String::from("managed sessions are required")));
    }
    let started = Instant::now();
    let millis = arguments
        .managed
        .iter()
        .map(|input| input.deadline_millis)
        .min()
        .unwrap_or(0);
    let limit = Duration::from_millis(millis);
    let mut frames = Vec::new();
    let mut sessions = Vec::new();
    for session in &arguments.managed {
        let mut input: Input = transcode(session).map_err(|source| failure(source.to_string()))?;
        input.deadline_millis =
            u64::try_from(limit.saturating_sub(started.elapsed()).as_millis()).unwrap_or(0);
        if input.deadline_millis == 0 {
            return Err(failure(String::from("managed transcript deadline expired")));
        }
        let mut answer: Observation = serde_json::from_value(Op::run(&input)?)
            .map_err(|source| failure(source.to_string()))?;
        let mut previous = None;
        for (index, frame) in answer.frames.iter_mut().enumerate() {
            if session.normalize_warmup {
                normalize_warmup(frame, index, &mut previous)?;
            }
            frame.sort_all_objects();
            frames.push(json!({"direction":"provider_to_client", "action":"expect",
                "text":serde_json::to_string(frame).map_err(|source| failure(source.to_string()))?}));
        }
        sessions.push(answer.effects);
    }
    Ok(
        json!({"handshake":{"status":101,"negotiated_subprotocol":null},
        "frames":frames,"close":null,"managed":sessions}),
    )
}

// Validate generated fields before canonicalizing them so random IDs and clocks cannot hide malformed acknowledgements.
fn normalize_warmup(
    value: &mut Value,
    index: usize,
    previous: &mut Option<String>,
) -> Result<(), Box<HelperError>> {
    let record: Warmup =
        serde_json::from_value(value.clone()).map_err(|source| failure(source.to_string()))?;
    let response = record.response;
    let valid_id = response.id.strip_prefix("resp_warmup_").is_some_and(|hex| {
        hex.len() == 32
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if !valid_id
        || response.created_at < 0
        || (!index.is_multiple_of(2) && previous.as_ref() != Some(&response.id))
    {
        return Err(failure(String::from("invalid generated warmup fields")));
    }
    *previous = Some(response.id);
    if let Some(id) = value.pointer_mut("/response/id") {
        *id = json!("resp_warmup_0123456789abcdef0123456789abcdef");
    }
    if let Some(timestamp) = value.pointer_mut("/response/created_at") {
        *timestamp = json!(0_i64);
    }
    Ok(())
}

#[path = "service/tests.rs"]
mod tests;
