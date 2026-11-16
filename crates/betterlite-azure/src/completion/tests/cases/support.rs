use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_prompts::builtin::Message;
use betterlite_streaming::stream_chunk_shaping::records::{JsonObject, StreamChunk};
use futures_util::StreamExt as _;
use secrecy::SecretString;
use serde_json::Number;

use crate::completion::arguments::{CompletionCall, Runtime};
use crate::completion::client::ClientHandle;
use crate::completion::error::AzureTextError;
use crate::completion::handler::{TextCompletionOutcome, completion};
use crate::completion::response::ChatReply;
use crate::completion::tests::fixture::{
    Answer, CLOCK, Fixed, Recorder, ScriptedSource, ScriptedTransport, clock, identifier, object,
    run, stream_id, stream_now,
};
use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::projection::{ClientKind, LiteLlmParams};

pub(super) const MODEL: &str = "gpt-35-turbo-instruct";
pub(super) const API_BASE: &str = "https://unit.test";
pub(super) const RESPONSE: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-35-turbo-instruct","choices":[{"text":"Hello!","index":0,"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

fn sse(piece: &str, finish: &str) -> String {
    format!(
        "data: {{\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":{CLOCK},\"model\":\"{MODEL}\",\"choices\":[{{\"text\":\"{piece}\",\"index\":0,\"finish_reason\":{finish}}}]}}\n\n"
    )
}

pub(super) fn stream_items() -> Vec<String> {
    vec![
        sse("Hi ", "null"),
        sse("there", "\"stop\""),
        "data: [DONE]\n\n".to_owned(),
    ]
}

pub(super) fn text_messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"Hello"}]"#).unwrap()
}

pub(super) fn parts_messages() -> Vec<Message> {
    serde_json::from_str(
        r#"[{"role":"system","content":[{"type":"text","text":"Sys "}]},{"role":"user","content":"Hi"}]"#,
    )
    .unwrap()
}

pub(super) fn buffered() -> Answer {
    Answer::Buffered(RESPONSE.to_owned())
}

pub(super) fn sync_buffered(params: &str) -> Scenario {
    Scenario::new(ClientKind::AzureOpenAi, buffered(), params)
}

pub(super) fn async_buffered(params: &str) -> Scenario {
    Scenario::new(ClientKind::AsyncAzureOpenAi, buffered(), params)
}

pub(super) fn sync_stream(params: &str) -> Scenario {
    Scenario::new(
        ClientKind::AzureOpenAi,
        Answer::Stream(stream_items()),
        params,
    )
}

pub(super) fn async_stream(params: &str) -> Scenario {
    Scenario::new(
        ClientKind::AsyncAzureOpenAi,
        Answer::Stream(stream_items()),
        params,
    )
}

pub(super) struct Parts {
    pub(super) model: Option<String>,
    pub(super) api_base: Option<String>,
    pub(super) api_version: Option<String>,
    pub(super) key: Option<SecretString>,
    pub(super) token: Option<SecretString>,
    pub(super) headers: Option<SecretHeaders>,
    pub(super) supplied: Option<ClientHandle<ScriptedTransport>>,
    pub(super) litellm_params: LiteLlmParams,
}

impl Default for Parts {
    fn default() -> Self {
        Self {
            model: Some(MODEL.to_owned()),
            api_base: Some(API_BASE.to_owned()),
            api_version: Some("2024-02-01".to_owned()),
            key: Some(SecretString::from("sk-unit")),
            token: None,
            headers: None,
            supplied: None,
            litellm_params: LiteLlmParams::default(),
        }
    }
}

impl Parts {
    pub(super) fn call<'call>(
        &'call self,
        messages: Option<&'call [Message]>,
        params: &'call mut Extra,
        acompletion: bool,
    ) -> CompletionCall<'call, ScriptedTransport> {
        CompletionCall {
            model: self.model.as_deref(),
            messages,
            api_key: self.key.as_ref(),
            api_base: self.api_base.as_deref(),
            api_version: self.api_version.as_deref(),
            azure_ad_token: self.token.as_ref(),
            azure_ad_token_provider: None,
            timeout: Some(30.0),
            optional_params: params,
            litellm_params: self.litellm_params.clone(),
            cloudflare_params: CloudflareParams::default(),
            acompletion,
            headers: self.headers.as_ref(),
            supplied: self.supplied.clone(),
        }
    }
}

pub(super) struct Scenario {
    pub(super) parts: Parts,
    pub(super) messages: Option<Vec<Message>>,
    pub(super) params: Extra,
    pub(super) source: ScriptedSource,
    pub(super) acompletion: bool,
    pub(super) fail_pre: bool,
}

pub(super) struct Run {
    pub(super) reply: Option<ChatReply>,
    pub(super) chunks: Vec<StreamChunk>,
    pub(super) error: Option<AzureTextError>,
    pub(super) source: ScriptedSource,
    pub(super) hooks: Recorder,
}

pub(super) fn runtime(fixed: &Fixed) -> Runtime<'_> {
    Runtime {
        prompts: &fixed.factory,
        now: fixed.now,
        identifier: &identifier,
        clock: &clock,
        stream_now,
        stream_id,
    }
}

async fn collect(mut stream: CompletionStream) -> Vec<StreamChunk> {
    let mut collected = Vec::new();
    while let Some(item) = stream.next().await {
        collected.push(item.unwrap());
    }
    collected
}

impl Scenario {
    pub(super) fn new(kind: ClientKind, answer: Answer, params: &str) -> Self {
        Self {
            parts: Parts::default(),
            messages: Some(text_messages()),
            params: object(params),
            source: ScriptedSource::new(kind, answer),
            acompletion: matches!(kind, ClientKind::AsyncAzureOpenAi),
            fail_pre: false,
        }
    }

    pub(super) fn execute(mut self) -> Run {
        let fixed = Fixed::default();
        let mut hooks = Recorder {
            pre: Vec::new(),
            post: Vec::new(),
            fail_pre: self.fail_pre,
            hidden: JsonObject::new(),
        };
        let mut call =
            self.parts
                .call(self.messages.as_deref(), &mut self.params, self.acompletion);
        let outcome = run(completion(
            &self.source,
            &mut call,
            &runtime(&fixed),
            &mut hooks,
        ));
        let mut result = Run {
            reply: None,
            chunks: Vec::new(),
            error: None,
            source: self.source,
            hooks,
        };
        match outcome {
            Ok(TextCompletionOutcome::Buffered(reply)) => result.reply = Some(reply),
            Ok(TextCompletionOutcome::Stream(stream)) => result.chunks = run(collect(stream)),
            Err(error) => result.error = Some(error),
        }
        result
    }
}

impl Run {
    pub(super) fn reply_json(&self) -> Json {
        let value = serde_json::to_value(self.reply.as_ref().unwrap()).unwrap();
        serde_json::from_value(value).unwrap()
    }

    pub(super) fn failure(&self) -> &AzureTextError {
        self.error.as_ref().unwrap()
    }

    pub(super) fn pre(&self, path: &[&str]) -> Json {
        nested(&self.hooks.pre[0], path)
    }

    pub(super) fn create(&self, name: &str) -> Json {
        nested(&self.source.creates()[0], &["data", name])
    }

    pub(super) fn contents(&self) -> Vec<String> {
        self.chunks
            .iter()
            .flat_map(|chunk| chunk.choices.iter())
            .filter_map(|choice| choice.delta.content.clone())
            .collect()
    }
}

pub(super) fn field(value: &Json, name: &str) -> Json {
    match value {
        Json::Object(fields) => fields.get(name).cloned().unwrap_or(Json::Null),
        Json::List(items) => name
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get(index).cloned())
            .unwrap_or(Json::Null),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => Json::Null,
    }
}

pub(super) fn nested(value: &Json, names: &[&str]) -> Json {
    names
        .iter()
        .fold(value.clone(), |current, name| field(&current, name))
}

pub(super) fn at(value: &Json, path: &str) -> Json {
    let names: Vec<&str> = path.split('.').collect();
    nested(value, &names)
}

pub(super) fn text(value: &Json) -> String {
    if matches!(value, Json::Null) {
        return String::new();
    }
    let Json::Text(content) = value else {
        panic!("not text: {value:?}");
    };
    content.clone()
}

pub(super) fn number(value: &Json) -> i64 {
    let Json::Number(count) = value else {
        panic!("not a number: {value:?}");
    };
    count.as_i64().unwrap()
}

pub(super) fn float(value: f64) -> Json {
    Json::Number(Number::from_f64(value).unwrap())
}

pub(super) fn count(value: i64) -> Json {
    Json::Number(Number::from(value))
}
