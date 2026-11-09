use std::collections::BTreeMap;

use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_prompts::builtin::Message;
use betterlite_prompts::watsonx::{apply_prompt_template, messages_to_prompt};
use betterlite_provider::watsonx_text::WatsonxTextConfig;
use betterlite_provider::watsonx_text::reply::transform_response;
use betterlite_provider::watsonx_text::request::{
    EU_REGIONS, SPECIAL_AUTH_PARAMS, SUPPORTED_OPENAI_PARAMS, US_REGIONS, complete_url,
    is_watsonx_text_param, map_openai_params, map_special_auth_params,
};
use betterlite_provider::watsonx_text::stream::decode_stream_line;
use chrono::DateTime;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

use self::script::{
    MapCase, NOW_SECONDS, PayloadRequest, PromptCase, ReplyRequest, UrlRequest, environment,
    factory, runtime,
};

mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    Prompts {
        cases: Vec<PromptCase>,
        messages: Vec<Message>,
        #[serde(default)]
        templates: BTreeMap<String, String>,
    },
    MapParams {
        cases: Vec<MapCase>,
        probe: Vec<String>,
    },
    Payload {
        requests: Vec<PayloadRequest>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        templates: BTreeMap<String, String>,
    },
    Url {
        requests: Vec<UrlRequest>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Reply {
        replies: Vec<ReplyRequest>,
    },
    StreamLine {
        lines: Vec<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::Prompts {
                cases,
                messages,
                templates,
            } => prompts(cases, messages, templates),
            Input::MapParams { cases, probe } => Ok(map_params(cases, probe)),
            Input::Payload {
                requests,
                env,
                templates,
            } => payload(requests, &environment(env), templates),
            Input::Url { requests, env } => Ok(urls(requests, &environment(env))),
            Input::Reply { replies } => Ok(json!({
                "replies": replies.iter().map(reply).collect::<Vec<_>>(),
            })),
            Input::StreamLine { lines } => Ok(json!({
                "chunks": lines.iter().map(|line| json!(decode_stream_line(line))).collect::<Vec<_>>(),
            })),
        }
    }
}

fn prompts(
    cases: &[PromptCase],
    messages: &[Message],
    templates: &BTreeMap<String, String>,
) -> HelperResult {
    let factory = factory(templates);
    let runtime = runtime()?;
    let now = DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default();
    let prompts: Vec<Value> = cases
        .iter()
        .map(|case| {
            let prompt = case.provider.as_deref().map_or_else(
                || runtime.block_on(apply_prompt_template(&factory, &case.model, messages, now)),
                |provider| {
                    runtime.block_on(messages_to_prompt(
                        &factory,
                        &case.model,
                        messages,
                        provider,
                        now,
                    ))
                },
            );
            prompt.map_or_else(
                |_error| json!({"ok": false, "status": 500_u16}),
                |text| text.map_or(Value::Null, Value::String),
            )
        })
        .collect();
    Ok(json!({"prompts": prompts}))
}

fn map_params(cases: &[MapCase], probe: &[String]) -> Value {
    let results: Vec<Value> = cases
        .iter()
        .map(|case| {
            let mut options = case.optional.clone();
            map_openai_params(&case.non_default, &mut options);
            let mut auth = case.optional.clone();
            map_special_auth_params(&case.non_default, &mut auth);
            json!({"options": options, "auth": auth})
        })
        .collect();
    json!({
        "results": results,
        "supported": SUPPORTED_OPENAI_PARAMS,
        "text_param": probe.iter().map(|name| is_watsonx_text_param(name)).collect::<Vec<_>>(),
        "special_auth": BTreeMap::from(SPECIAL_AUTH_PARAMS),
        "eu_regions": EU_REGIONS,
        "us_regions": US_REGIONS,
    })
}

fn payload(
    requests: &[PayloadRequest],
    environment: &WatsonxEnvironment,
    templates: &BTreeMap<String, String>,
) -> HelperResult {
    let prompts = factory(templates);
    let config = WatsonxTextConfig {
        environment,
        prompts: &prompts,
        now: DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default(),
    };
    let runtime = runtime()?;
    let bodies: Vec<Value> = requests
        .iter()
        .map(|request| {
            runtime
                .block_on(config.transform_request(
                    &request.model,
                    &request.messages,
                    request.options.clone(),
                ))
                .map_or_else(
                    |error| json!({"ok": false, "status": error.status().unwrap_or(500)}),
                    |body| json!({"ok": true, "body": body}),
                )
        })
        .collect();
    Ok(json!({"bodies": bodies}))
}

fn urls(requests: &[UrlRequest], environment: &WatsonxEnvironment) -> Value {
    let urls: Vec<Value> = requests
        .iter()
        .map(|request| {
            let mut options = request.options.clone();
            match complete_url(
                environment,
                request.api_base.as_deref(),
                &request.model,
                &mut options,
                request.stream,
            ) {
                Ok(url) => json!({"ok": true, "url": url, "options": options}),
                Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
            }
        })
        .collect();
    json!({"urls": urls})
}

fn reply(request: &ReplyRequest) -> Value {
    let fallback = DateTime::from_timestamp(NOW_SECONDS, 0).unwrap_or_default();
    let timestamp = request
        .now
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .unwrap_or(fallback);
    match transform_response(request.body.as_bytes(), timestamp) {
        Ok(response) => json!({
            "ok": true,
            "text": response.text,
            "finish_reason": response.finish_reason,
            "created": response.created,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
            },
        }),
        Err(error) => json!({"ok": false, "status": error.status().unwrap_or(500)}),
    }
}
