use crate::helpers::{Helper, HelperError, HelperResult};
use betterlite_openai::codestral_call::log::{CallLogger, PreCall, StreamPostCall};
use betterlite_openai::codestral_call::plan::{CodestralRequest, Mode, prepare};
use betterlite_openai::codestral_records::options::CodestralSettings;
use betterlite_openai::codestral_records::reply::{PostCall, PostCallLogger};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::factory::PromptFactory;
use betterlite_prompts::factory::custom::CustomPromptTemplate;
use betterlite_prompts::factory::hub::{Hub, builtin_entries};
use chrono::{DateTime, TimeZone as _, Utc};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;
mod tests;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    settings: CodestralSettings,
    calls: Vec<Call>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    mode: Mode,
    api_key: Option<String>,
    model: String,
    messages: Vec<Message>,
    timeout: f64,
    headers: Headers,
    api_base: Option<String>,
    custom_prompt_dict: BTreeMap<String, CustomPromptTemplate>,
    optional_params: Map<String, Value>,
}
#[derive(Default)]
struct Recorder {
    events: Vec<Value>,
}
impl PostCallLogger for Recorder {
    fn post_call(&mut self, _call: PostCall<'_>) {}
}
impl CallLogger for Recorder {
    fn pre_call(&mut self, call: PreCall<'_>) {
        self.events.push(json!({
            "phase": "pre_call",
            "input": call.input,
            "api_key": call.api_key.map(|key| key.expose_secret().to_owned()),
            "additional_args": {
                "complete_input_dict": call.body,
                "headers": call.headers,
                "api_base": call.api_base,
                "acompletion": call.acompletion,
            },
        }));
    }
    fn stream_post_call(&mut self, _call: StreamPostCall<'_>) {}
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = Input;
    fn run(input: &Self::Input) -> HelperResult {
        let prompts = PromptFactory {
            cache: Mutex::new(builtin_entries()),
            hub: Hub::Unreachable,
        };
        let now = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                Box::new(HelperError {
                    class: String::from("Runtime"),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message: error.to_string(),
                })
            })?;
        let calls: Vec<Value> = input
            .calls
            .iter()
            .map(|call| run_call(call, &input.settings, &prompts, now, &runtime))
            .collect();
        Ok(Value::Array(calls))
    }
}
fn run_call(
    call: &Call,
    settings: &CodestralSettings,
    prompts: &PromptFactory,
    now: DateTime<Utc>,
    runtime: &tokio::runtime::Runtime,
) -> Value {
    let secret = call.api_key.clone().map(SecretString::from);
    let Ok(timeout) = Duration::try_from_secs_f64(call.timeout) else {
        return json!({"outcome": "error"});
    };
    let request = CodestralRequest {
        api_key: secret.as_ref(),
        headers: &call.headers,
        model: &call.model,
        messages: &call.messages,
        api_base: call.api_base.as_deref(),
        custom_prompts: &call.custom_prompt_dict,
        optional_params: &call.optional_params,
        settings,
        mode: call.mode,
        timeout,
    };
    let mut recorder = Recorder::default();
    let Ok(plan) = runtime.block_on(prepare(&request, prompts, now, &mut recorder)) else {
        return json!({"outcome": "error"});
    };
    json!({
        "mode": plan.mode,
        "request": {"url": plan.url, "headers": plan.headers, "body": plan.body},
        "provider": plan.wrapper,
        "timeout": plan.timeout.map(|duration| duration.as_secs_f64()),
        "events": recorder.events,
    })
}
