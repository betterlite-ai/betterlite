use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::args::{AdditionalArgs, RawBody};
use super::env::{LogLevel, PrePostEnv};
use super::masking::{mask_api_base, masked_headers};
use super::request::{CurlRequest, curl_command};

const CURL_COLOR_START: &str = "\x1b[92m";
const CURL_COLOR_END: &str = "\x1b[0m";

// The record carries the masked base and headers only, so a log reader never sees the credential that signed the call
fn structured_fields(args: &AdditionalArgs, env: &PrePostEnv<'_>) -> IndexMap<String, Json> {
    let api_base = mask_api_base(args.api_base.as_deref().unwrap_or_default());
    let empty_headers = IndexMap::new();
    let mut fields = IndexMap::from([(
        "api_base".to_owned(),
        Json::Text(env.redactor.redact_string(&api_base)),
    )]);
    for (name, value) in masked_headers(args.headers.as_ref().unwrap_or(&empty_headers), false) {
        let shown = if let Json::Text(text) = &value {
            Json::Text(env.redactor.redact_string(text))
        } else {
            value
        };
        fields.insert(name, shown);
    }
    fields
}

pub fn print_llm_call_debugging_log(
    request_debug: bool,
    args: &AdditionalArgs,
    env: &PrePostEnv<'_>,
) {
    if !(env.debug.verbose || request_debug) {
        return;
    }
    // warning keeps the line visible in every environment when the request itself asked for debugging
    let level = if request_debug {
        LogLevel::Warning
    } else {
        LogLevel::Debug
    };
    if env.debug.json_logs {
        env.emit(
            level,
            "POST Request Sent from LiteLLM",
            structured_fields(args, env),
        );
        return;
    }
    let empty_body = RawBody::default();
    let empty_headers = IndexMap::new();
    let command = curl_command(&CurlRequest {
        api_base: args.api_base.as_deref().unwrap_or_default(),
        headers: args.headers.as_ref().unwrap_or(&empty_headers),
        request_str: args.request_str.as_deref(),
        data: args.complete_input_dict.as_ref().unwrap_or(&empty_body),
    });
    env.emit(
        level,
        &format!("{CURL_COLOR_START}{command}{CURL_COLOR_END}\n"),
        IndexMap::new(),
    );
}
