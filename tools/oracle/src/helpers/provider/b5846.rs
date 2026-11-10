use std::collections::BTreeMap;

use async_trait::async_trait;
use betterlite_prompts::arguments::json::Object;
use betterlite_prompts::builtin::Message;
use betterlite_provider::predibase::config::{
    PredibaseConfig, get_complete_url, get_supported_openai_params, map_openai_params,
    validate_environment,
};
use betterlite_provider::predibase::decode::output_parser;
use betterlite_provider::predibase::error::get_error_class;
use betterlite_provider::predibase::reply::{ReplyInput, TokenCounter, transform_response};
use betterlite_provider::predibase::request::{CustomPrompt, RequestBody, RequestContext};
use chrono::DateTime;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::helpers::{Helper, HelperResult};

use self::script::{
    FIXED_NOW, HeaderRequest, MapCase, PayloadRequest, ReplyRequest, UrlRequest,
    default_max_new_tokens, factory, runtime,
};

mod script;

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub(crate) enum Input {
    MapParams {
        #[serde(default = "default_max_new_tokens")]
        max_new_tokens: i64,
        cases: Vec<MapCase>,
    },
    Payload {
        #[serde(default = "default_max_new_tokens")]
        max_new_tokens: i64,
        requests: Vec<PayloadRequest>,
        #[serde(default)]
        custom_prompt_dict: BTreeMap<String, CustomPrompt>,
    },
    Url {
        requests: Vec<UrlRequest>,
    },
    Headers {
        requests: Vec<HeaderRequest>,
    },
    Reply {
        replies: Vec<ReplyRequest>,
    },
    OutputParser {
        texts: Vec<String>,
    },
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::MapParams {
                max_new_tokens,
                cases,
            } => Ok(map_params(*max_new_tokens, cases)),
            Input::Payload {
                max_new_tokens,
                requests,
                custom_prompt_dict,
            } => payload(*max_new_tokens, requests, custom_prompt_dict),
            Input::Url { requests } => Ok(urls(requests)),
            Input::Headers { requests } => Ok(headers(requests)),
            Input::Reply { replies } => results(replies),
            Input::OutputParser { texts } => Ok(json!({
                "texts": texts.iter().map(|text| output_parser(text)).collect::<Vec<String>>(),
            })),
        }
    }
}

fn map_params(max_new_tokens: i64, cases: &[MapCase]) -> Value {
    let config = PredibaseConfig {
        details: true,
        max_new_tokens,
        return_full_text: false,
    };
    let results: Vec<Value> = cases
        .iter()
        .map(|case| {
            let mut options = Object::new();
            map_openai_params(&case.non_default, &mut options);
            json!({"options": options})
        })
        .collect();
    json!({
        "results": results,
        "supported": get_supported_openai_params(),
        "config": config.get_config(),
    })
}

fn payload(
    max_new_tokens: i64,
    requests: &[PayloadRequest],
    custom_prompt_dict: &BTreeMap<String, CustomPrompt>,
) -> HelperResult {
    let runtime = runtime()?;
    let config = PredibaseConfig {
        details: true,
        max_new_tokens,
        return_full_text: false,
    };
    let prompts = factory();
    let context = RequestContext {
        config: &config,
        prompts: &prompts,
        custom_prompts: custom_prompt_dict,
        now: DateTime::from_timestamp(FIXED_NOW, 0).unwrap_or_default(),
    };
    let bodies: Vec<Value> = requests
        .iter()
        .map(|request| {
            match runtime.block_on(context.transform_request(
                &request.model,
                &request.messages,
                &request.options,
            )) {
                Ok(body) => json!({"ok": true, "body": body}),
                Err(_error) => json!({"ok": false, "status": 500_u16}),
            }
        })
        .collect();
    Ok(json!({"bodies": bodies}))
}

fn urls(requests: &[UrlRequest]) -> Value {
    let urls: Vec<Value> = requests
        .iter()
        .map(|request| {
            match get_complete_url(
                request.tenant.as_deref(),
                request.api_base.as_deref(),
                &request.model,
                &request.options,
                request.stream,
            ) {
                Ok(url) => json!({"ok": true, "url": url}),
                Err(_error) => json!({"ok": false, "status": 500_u16}),
            }
        })
        .collect();
    json!({"urls": urls})
}

fn headers(requests: &[HeaderRequest]) -> Value {
    let mut found = Vec::new();
    for request in requests {
        let key = request
            .api_key
            .as_ref()
            .map(|value| SecretString::from(value.clone()));
        let entry = match validate_environment(key.as_ref(), &request.headers) {
            Ok(merged) => json!({"ok": true, "headers": merged}),
            Err(_error) => json!({"ok": false, "status": 500_u16}),
        };
        found.push(entry);
    }
    let class = get_error_class(
        "boom",
        418,
        &BTreeMap::from([(String::from("x-a"), String::from("1"))]),
    );
    json!({
        "requests": found,
        "error_class": {"status_code": class.status_code, "headers": class.headers},
    })
}

struct StandInCounter {
    fail_messages: bool,
    encoding_fails: bool,
}

#[async_trait]
impl TokenCounter for StandInCounter {
    async fn count_messages(&self, messages: &[Message]) -> Option<i64> {
        (!self.fail_messages).then(|| i64::try_from(messages.len()).unwrap_or(i64::MAX))
    }

    async fn count_text(&self, text: &str) -> Option<i64> {
        (!self.encoding_fails).then(|| i64::try_from(text.chars().count()).unwrap_or(i64::MAX))
    }
}

fn results(replies: &[ReplyRequest]) -> HelperResult {
    let runtime = runtime()?;
    let results: Vec<Value> = replies
        .iter()
        .map(|reply| one_reply(&runtime, reply))
        .collect();
    Ok(json!({"replies": results}))
}

fn one_reply(runtime: &tokio::runtime::Runtime, reply: &ReplyRequest) -> Value {
    let request_body = RequestBody {
        inputs: None,
        parameters: reply.parameters.clone(),
    };
    let counter = StandInCounter {
        fail_messages: reply.counter_fails,
        encoding_fails: reply.encoding_fails,
    };
    let input = ReplyInput {
        body: reply.body.as_bytes(),
        status: reply.status,
        headers: &reply.headers,
        request_body: &request_body,
        options: &reply.options,
        messages: &reply.messages,
        model: &reply.model,
        created: reply.now.unwrap_or(FIXED_NOW),
    };
    match runtime.block_on(transform_response(&input, &counter)) {
        Ok(response) => json!({
            "ok": true,
            "model": response.model,
            "created": response.created,
            "usage": {
                "prompt_tokens": response.usage.prompt_tokens,
                "completion_tokens": response.usage.completion_tokens,
                "total_tokens": response.usage.total_tokens,
            },
            "additional_headers": response.additional_headers,
            "choices": response.choices.iter().map(|choice| json!({
                "index": choice.index,
                "content": choice.content,
                "finish_reason": choice.finish_reason,
                "logprobs": choice.logprobs,
                "logprob": choice.private_logprob,
            })).collect::<Vec<Value>>(),
        }),
        Err(error) => json!({"ok": false, "status": error.status_code}),
    }
}
