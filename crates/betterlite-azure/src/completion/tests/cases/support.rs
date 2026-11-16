use betterlite_models::decisions::{Extra, Json};
use betterlite_openai::text_completion_transport::logging::SecretHeaders;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use betterlite_prompts::builtin::Message;
use betterlite_streaming::stream_chunk_shaping::records::StreamChunk;
use futures_util::StreamExt as _;
use secrecy::SecretString;

use crate::completion::arguments::{CompletionCall, Runtime, SentCall};
use crate::completion::client::ClientHandle;
use crate::completion::error::{AzureTextError, AzureTextFailure};
use crate::completion::handler::{AzureTextCompletion, TextCompletionOutcome};
use crate::completion::request::Retries;
use crate::completion::response::ChatReply;
use crate::completion::tests::fixture::{
    Answer, CLOCK, Fixed, Recorder, ScriptedSource, ScriptedTransport, clock, identifier, object,
    run, stream_id, stream_now,
};
use crate::endpoints::cloudflare::CloudflareParams;
use crate::endpoints::projection::LiteLlmParams;

pub const MODEL: &str = "gpt-35-turbo-instruct";
pub const API_BASE: &str = "https://unit.test";

pub const RESPONSE: &str = r#"{"id":"cmpl-1","object":"text_completion","created":1700000000,"model":"gpt-35-turbo-instruct","choices":[{"text":"Hello!","index":0,"logprobs":null,"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}"#;

pub fn stream_items() -> Vec<String> {
    vec![
        format!(
            "data: {{\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":{CLOCK},\"model\":\"{MODEL}\",\"choices\":[{{\"text\":\"Hel\",\"index\":0,\"finish_reason\":null}}]}}\n\n"
        ),
        format!(
            "data: {{\"id\":\"cmpl-1\",\"object\":\"text_completion\",\"created\":{CLOCK},\"model\":\"{MODEL}\",\"choices\":[{{\"text\":\"lo\",\"index\":0,\"finish_reason\":\"stop\"}}]}}\n\n"
        ),
        "data: [DONE]\n\n".to_owned(),
    ]
}

pub fn text_messages() -> Vec<Message> {
    serde_json::from_str(r#"[{"role":"user","content":"Hello"}]"#).unwrap()
}

pub fn parts_messages() -> Vec<Message> {
    serde_json::from_str(
        r#"[{"role":"system","content":[{"type":"text","text":"Sys "}]},{"role":"user","content":"Hi"}]"#,
    )
    .unwrap()
}

pub struct Parts {
    pub model: Option<String>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub key: Option<SecretString>,
    pub token: Option<SecretString>,
    pub headers: Option<SecretHeaders>,
    pub supplied: Option<ClientHandle<ScriptedTransport>>,
    pub litellm_params: LiteLlmParams,
}

impl Parts {
    pub fn new() -> Self {
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

    pub fn with_client(mut self, handle: ClientHandle<ScriptedTransport>) -> Self {
        self.supplied = Some(handle);
        self
    }

    pub fn call<'a>(
        &'a self,
        messages: Option<&'a [Message]>,
        params: &'a mut Extra,
        acompletion: bool,
    ) -> CompletionCall<'a, ScriptedTransport> {
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

impl Default for Parts {
    fn default() -> Self {
        Self::new()
    }
}

pub fn runtime(fixed: &Fixed) -> Runtime<'_> {
    Runtime {
        prompts: &fixed.factory,
        now: fixed.now,
        identifier: &identifier,
        clock: &clock,
        stream_now,
        stream_id,
    }
}

pub fn completion(
    source: ScriptedSource,
    call: CompletionCall<'_, ScriptedTransport>,
    fixed: &Fixed,
    recorder: &mut Recorder,
) -> Result<TextCompletionOutcome, AzureTextError> {
    let handler = AzureTextCompletion::new(source);
    run(handler.completion(call, &runtime(fixed), recorder))
}

pub fn streaming(
    source: ScriptedSource,
    call: CompletionCall<'_, ScriptedTransport>,
    sent: &mut SentCall<'_, ScriptedTransport>,
    fixed: &Fixed,
    recorder: &mut Recorder,
) -> Result<CompletionStream, AzureTextFailure> {
    let handler = AzureTextCompletion::new(source);
    run(handler.streaming(sent, &call, &runtime(fixed), recorder))
}

pub fn sent<'a>(
    model: &str,
    data: Extra,
    litellm_params: &'a LiteLlmParams,
) -> SentCall<'a, ScriptedTransport> {
    let mut empty = Extra::new();
    SentCall {
        model: model.to_owned(),
        data,
        prompt: String::new(),
        max_retries: Retries::pop(&mut empty),
        litellm_params,
        supplied: None,
    }
}

pub async fn chunks(stream: CompletionStream) -> Vec<StreamChunk> {
    let mut stream = stream;
    let mut collected = Vec::new();
    while let Some(item) = stream.next().await {
        collected.push(item.unwrap());
    }
    collected
}

pub fn buffered() -> Answer {
    Answer::Buffered(RESPONSE.to_owned())
}

pub fn contents(chunks: &[StreamChunk]) -> Vec<String> {
    chunks
        .iter()
        .flat_map(|chunk| chunk.choices.iter())
        .filter_map(|choice| choice.delta.content.clone())
        .collect()
}

pub fn data_field(records: &[Json], name: &str) -> Json {
    match &records[0] {
        Json::Object(fields) => match fields.get("data") {
            Some(Json::Object(data)) => data.get(name).cloned().unwrap_or(Json::Null),
            _ => Json::Null,
        },
        _ => Json::Null,
    }
}

pub fn object_json(source: &str) -> Extra {
    object(source)
}

pub fn reply_json(reply: &ChatReply) -> Json {
    serde_json::from_value(serde_json::to_value(reply).unwrap()).unwrap()
}

pub fn field(value: &Json, name: &str) -> Json {
    match value {
        Json::Object(fields) => fields.get(name).cloned().unwrap_or(Json::Null),
        Json::List(items) => name
            .parse::<usize>()
            .ok()
            .and_then(|index| items.get(index).cloned())
            .unwrap_or(Json::Null),
        _ => Json::Null,
    }
}

pub fn nested(value: &Json, names: &[&str]) -> Json {
    names
        .iter()
        .fold(value.clone(), |current, name| field(&current, name))
}

pub fn text(value: &Json) -> String {
    match value {
        Json::Text(text) => text.clone(),
        Json::Null => String::new(),
        other => other.to_string(),
    }
}

pub fn number(value: &Json) -> i64 {
    match value {
        Json::Number(value) => value.as_i64().unwrap_or_default(),
        _ => i64::default(),
    }
}
