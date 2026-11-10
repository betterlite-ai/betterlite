use std::collections::BTreeMap;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::types::ProxyTrust;
use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::aleph_alpha::adapter::{
    AlephAlphaAdapter, CallPhase, CompletionOutcome, CompletionReply,
};
use betterlite_openai::aleph_alpha::config::AlephAlphaConfig;
use betterlite_openai::aleph_alpha::error::AlephAlphaError;
use betterlite_openai::aleph_alpha::lines::LineStream;
use betterlite_openai::aleph_alpha::request::CompletionInput;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::chat::response::ChatChoice;
use betterlite_openai::openai_base_config::Object;
use betterlite_tokenizer::engine::Engine;
use betterlite_tokenizer::engine::assets;
use betterlite_tokenizer::engine::tiktoken::{Encoding, TiktokenEngine};
use futures_util::StreamExt as _;
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::Value;
use url::Url;

use crate::helpers::{Helper, HelperError, HelperResult};

mod server;

use server::{Captured, Server};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    model: String,
    messages: Vec<ChatMessage>,
    api_base: String,
    api_key: Option<String>,
    #[serde(default)]
    optional_params: Object,
    #[serde(default)]
    defaults: Object,
    max_tokens: i64,
    created: i64,
    reply: Reply,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    status: u16,
    body: String,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| failure("OSError", &error))?;
        runtime.block_on(execute(input))
    }
}

async fn execute(input: &Input) -> HelperResult {
    let config = AlephAlphaConfig::new(input.max_tokens, input.defaults.clone())
        .map_err(|error| failure("ConfigError", &error))?;
    let server = Server::start(input.reply.status, input.reply.body.clone().into_bytes()).await?;
    let mut url = Url::parse(&server.base).map_err(|error| failure("ValueError", &error))?;
    let given = Url::parse(&input.api_base).map_err(|error| failure("ValueError", &error))?;
    url.set_path(given.path());
    url.set_query(given.query());
    let key = input.api_key.as_deref().map(SecretString::from);
    let adapter = AlephAlphaAdapter {
        client: client()?,
        tokenizer: cl100k()?,
        timeout: Duration::from_mins(1),
    };
    let call = CompletionInput {
        model: &input.model,
        messages: &input.messages,
        api_base: url.as_str(),
        api_key: key.as_ref(),
        optional_params: &input.optional_params,
        config: &config,
        created: input.created,
    };
    let mut phases = Vec::new();
    let mut sink = |phase: CallPhase<'_>| phases.push(phase_value(&phase));
    let outcome = adapter.completion(&call, &mut sink).await;
    let captured = server.finish().await?;
    outcome_value(outcome, captured.as_ref(), phases).await
}

async fn outcome_value(
    outcome: Result<CompletionOutcome, AlephAlphaError>,
    captured: Option<&Captured>,
    phases: Vec<Value>,
) -> HelperResult {
    let mut output = Object::new();
    output.insert(String::from("ok"), Value::Bool(outcome.is_ok()));
    output.insert(
        String::from("status"),
        outcome
            .as_ref()
            .err()
            .and_then(AlephAlphaError::status_code)
            .map_or(Value::Null, Value::from),
    );
    let mut reply_field = Value::Null;
    let mut lines_field = Value::Null;
    let mut tokens_field = Value::Array(Vec::new());
    match outcome {
        Ok(CompletionOutcome::Reply(reply)) => {
            reply_field = reply_value(&reply);
            tokens_field = tokens_value(captured, &reply);
        }
        Ok(CompletionOutcome::Stream(stream)) => lines_field = lines_value(stream).await?,
        Err(_) => {}
    }
    output.insert(String::from("reply"), reply_field);
    output.insert(String::from("lines"), lines_field);
    output.insert(String::from("tokens"), tokens_field);
    request_value(&mut output, captured);
    output.insert(String::from("log"), Value::Array(phases));
    Ok(Value::Object(output))
}

fn request_value(output: &mut Object, captured: Option<&Captured>) {
    let Some(found) = captured else {
        output.insert(String::from("path"), Value::Null);
        output.insert(String::from("query"), Value::Null);
        output.insert(String::from("headers"), Value::Null);
        output.insert(String::from("body"), Value::Null);
        return;
    };
    output.insert(String::from("path"), Value::from(found.path.clone()));
    output.insert(String::from("query"), Value::from(found.query.clone()));
    let mut headers = Object::new();
    for name in ["accept", "content-type", "authorization"] {
        if let Some(value) = found.headers.get(name) {
            headers.insert(name.to_owned(), Value::from(value.clone()));
        }
    }
    output.insert(String::from("headers"), Value::Object(headers));
    output.insert(String::from("body"), found.body.clone());
}

fn reply_value(reply: &CompletionReply) -> Value {
    let choices: Vec<Value> = reply.choices.iter().map(choice_value).collect();
    let mut usage = Object::new();
    usage.insert(
        String::from("prompt_tokens"),
        Value::from(reply.usage.prompt_tokens),
    );
    usage.insert(
        String::from("completion_tokens"),
        Value::from(reply.usage.completion_tokens),
    );
    usage.insert(
        String::from("total_tokens"),
        Value::from(reply.usage.total_tokens),
    );
    let mut value = Object::new();
    value.insert(String::from("choices"), Value::Array(choices));
    value.insert(String::from("created"), Value::from(reply.created));
    value.insert(String::from("model"), Value::from(reply.model.clone()));
    value.insert(String::from("usage"), Value::Object(usage));
    Value::Object(value)
}

fn choice_value(choice: &ChatChoice) -> Value {
    let mut fields = Object::new();
    fields.insert(String::from("index"), Value::from(choice.index));
    fields.insert(
        String::from("content"),
        choice
            .message
            .content
            .clone()
            .map_or(Value::Null, Value::from),
    );
    fields.insert(
        String::from("finish_reason"),
        choice
            .finish_reason
            .clone()
            .map_or(Value::Null, Value::from),
    );
    fields.insert(
        String::from("native_finish_reason"),
        choice
            .provider_specific_fields
            .get("native_finish_reason")
            .cloned()
            .unwrap_or(Value::Null),
    );
    Value::Object(fields)
}

fn tokens_value(captured: Option<&Captured>, reply: &CompletionReply) -> Value {
    let prompt = captured
        .and_then(|found| found.body.get("prompt"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let first = reply
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();
    Value::Array(vec![Value::from(prompt), Value::from(first)])
}

async fn lines_value(mut stream: LineStream) -> HelperResult {
    let mut collected = Vec::new();
    while let Some(item) = stream.next().await {
        let text = item.map_err(|error| failure("AlephAlphaError", &error))?;
        collected.push(Value::from(text));
    }
    Ok(Value::Array(collected))
}

fn phase_value(phase: &CallPhase<'_>) -> Value {
    let (name, prompt, key) = match phase {
        CallPhase::Pre {
            prompt, api_key, ..
        } => ("pre_call", *prompt, api_key.is_some()),
        CallPhase::Post {
            prompt, api_key, ..
        } => ("post_call", *prompt, api_key.is_some()),
    };
    let mut value = Object::new();
    value.insert(String::from("phase"), Value::from(name));
    value.insert(String::from("input"), Value::from(prompt));
    value.insert(String::from("api_key"), Value::Bool(key));
    Value::Object(value)
}

fn client() -> Result<ProxiedClient, Box<HelperError>> {
    let policy = ProxyPolicy::resolve(&ProxyTrust::default(), &BTreeMap::new())
        .map_err(|error| failure("ProxyError", &error))?;
    ProxiedClient::outbound(&Arc::new(policy)).map_err(|error| failure("HttpError", &error))
}

fn cl100k() -> Result<Engine, Box<HelperError>> {
    let data = assets::bundled(Encoding::Cl100kBase)
        .ok_or_else(|| failure("ValueError", &"cl100k_base is not bundled"))?;
    let engine = TiktokenEngine::from_bpe_file(Encoding::Cl100kBase, &data)
        .map_err(|error| failure("ValueError", &error))?;
    Ok(Engine::Tiktoken(Arc::new(engine)))
}

fn failure(class: &str, error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
