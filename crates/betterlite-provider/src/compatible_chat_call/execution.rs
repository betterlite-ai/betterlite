use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::chat::ChatMessage;
use betterlite_openai::compatible_line_stream::Mode;
use betterlite_openai::compatible_wire::{EndpointType, validate_environment};
use betterlite_openai::openai_base_config::Headers;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::error::CompatibleChatError;
use super::options::{headers_json, take_bool, take_object, take_text};
use super::plan::ChatPlan;
use super::send::{CompatibleChatOutcome, finish_chat, send_chat};
use super::stream::StreamingDecoder;

// litellm/custom_httpx/http_handler.py's _DEFAULT_TIMEOUT: a 600-second request with a 5-second connect
const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(600);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

// B2230's selection decides this: the OpenAI/GPT configs transform messages, every other config leaves them alone,
// so the provider binding passes Some only for those tags.
pub trait MessageTransformer {
    fn transform(
        &self,
        model: &str,
        messages: Vec<ChatMessage>,
    ) -> Result<Vec<ChatMessage>, Box<dyn std::error::Error + Send + Sync>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeoutFacts {
    pub total: Duration,
    pub connect: Duration,
}

impl TimeoutFacts {
    pub(crate) const DEFAULT: Self = Self {
        total: DEFAULT_TOTAL_TIMEOUT,
        connect: DEFAULT_CONNECT_TIMEOUT,
    };

    const fn uniform(timeout: Duration) -> Self {
        Self {
            total: timeout,
            connect: timeout,
        }
    }
}

pub struct CompatibleChatExecution<'source> {
    pub model: &'source str,
    pub messages: &'source [ChatMessage],
    pub custom_llm_provider: &'source str,
    pub api_base: Option<&'source str>,
    pub api_key: Option<&'source SecretString>,
    pub headers: Option<&'source Headers>,
    pub optional_params: &'source IndexMap<String, Json>,
    pub custom_endpoint: Option<bool>,
    pub timeout: Option<Duration>,
    pub mode: Mode,
    pub fake_stream: bool,
    pub transform_messages: Option<&'source dyn MessageTransformer>,
    pub streaming_decoder: Option<&'source dyn StreamingDecoder>,
}

// the option pops happen in the source's order, with the environment check between base_model and stream
struct Prepared {
    url: String,
    headers: Headers,
    stream: bool,
    extra_body: IndexMap<String, Json>,
    json_mode: bool,
    base_model: Option<String>,
    options: IndexMap<String, Json>,
}

// the body the pre-call log records and the one the send carries after the async stream override
struct Sent {
    url: String,
    headers: Headers,
    body: IndexMap<String, Json>,
    messages: Json,
    stream: bool,
    base_model: Option<String>,
    json_mode: bool,
}

impl<'source> CompatibleChatExecution<'source> {
    pub fn plan(
        &self,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<ChatPlan<'source>, CompatibleChatError> {
        let prepared = self.prepare()?;
        let sent = self.sent(prepared, state, env)?;
        let (send_timeout, construct_timeout) = self.timeouts();
        Ok(ChatPlan {
            mode: self.mode,
            stream: sent.stream,
            fake_stream: self.fake_stream,
            url: sent.url,
            headers: sent.headers,
            body: sent.body,
            messages: sent.messages,
            provider: self.custom_llm_provider.to_owned(),
            base_model: sent.base_model,
            json_mode: sent.json_mode,
            send_timeout,
            construct_timeout,
            decoder: self.streaming_decoder,
        })
    }

    fn prepare(&self) -> Result<Prepared, CompatibleChatError> {
        let mut options = self.optional_params.clone();
        // a truthy argument short-circuits the pop, so the option stays in the eventual body
        let custom_endpoint = if self.custom_endpoint == Some(true) {
            true
        } else {
            take_bool(&mut options, "custom_endpoint")?.unwrap_or(false)
        };
        let base_model = take_text(&mut options, "base_model")?;
        let (url, headers) = validate_environment(
            self.api_key,
            self.api_base,
            EndpointType::ChatCompletions,
            self.headers.cloned(),
            custom_endpoint,
        )?;
        let stream = take_bool(&mut options, "stream")?.unwrap_or(false);
        let extra_body = take_object(&mut options, "extra_body")?.unwrap_or_default();
        let json_mode = take_bool(&mut options, "json_mode")?.unwrap_or(false);
        options.shift_remove("max_retries");
        if !self.fake_stream {
            options.insert("stream".to_owned(), Json::Bool(stream));
        }
        Ok(Prepared {
            url,
            headers,
            stream,
            extra_body,
            json_mode,
            base_model,
            options,
        })
    }

    fn sent(
        &self,
        prepared: Prepared,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<Sent, CompatibleChatError> {
        let (messages, mut body) = self.body(prepared.options, prepared.extra_body)?;
        let logged = PreCall {
            input: messages.clone(),
            model: None,
            additional_args: AdditionalArgs {
                api_base: Some(prepared.url.clone()),
                headers: Some(headers_json(&prepared.headers)),
                complete_input_dict: Some(RawBody::Object(body.clone())),
                request_str: None,
                openai_sdk: None,
                passthrough: IndexMap::new(),
            },
        };
        pre_call(state, &logged, env).map_err(CompatibleChatError::PreCall)?;
        // acompletion_stream_function writes stream=true into the body after the pre-call log and before the send
        if self.mode == Mode::Async && prepared.stream {
            body.insert("stream".to_owned(), Json::Bool(true));
        }
        Ok(Sent {
            url: prepared.url,
            headers: prepared.headers,
            body,
            messages,
            stream: prepared.stream,
            base_model: prepared.base_model,
            json_mode: prepared.json_mode,
        })
    }

    fn body(
        &self,
        options: IndexMap<String, Json>,
        extra_body: IndexMap<String, Json>,
    ) -> Result<(Json, IndexMap<String, Json>), CompatibleChatError> {
        let messages = self.transformed_messages()?;
        let mut body = IndexMap::from([
            ("model".to_owned(), Json::Text(self.model.to_owned())),
            ("messages".to_owned(), messages.clone()),
        ]);
        body.extend(options);
        body.extend(extra_body);
        Ok((messages, body))
    }

    fn transformed_messages(&self) -> Result<Json, CompatibleChatError> {
        let transformed = match self.transform_messages {
            Some(transform) => transform
                .transform(self.model, self.messages.to_vec())
                .map_err(CompatibleChatError::Transform)?,
            None => self.messages.to_vec(),
        };
        let value = serde_json::to_value(&transformed).map_err(CompatibleChatError::Encode)?;
        serde_json::from_value(value).map_err(CompatibleChatError::Encode)
    }

    // acompletion_function builds the 600/5 default only when it got no timeout; make_call forwards none at all,
    // and the blocking handler is constructed with the caller's timeout and posts without one
    fn timeouts(&self) -> (Option<TimeoutFacts>, Option<TimeoutFacts>) {
        match (self.mode, self.stream) {
            (Mode::Async, true) => (None, None),
            (Mode::Async, false) => (
                Some(
                    self.timeout
                        .map_or(TimeoutFacts::DEFAULT, TimeoutFacts::uniform),
                ),
                None,
            ),
            (Mode::Sync, true) => (self.timeout.map(TimeoutFacts::uniform), None),
            (Mode::Sync, false) => (None, self.timeout.map(TimeoutFacts::uniform)),
        }
    }

    pub async fn execute(
        &self,
        client: &ProxiedClient,
        state: &mut LoggingState,
        env: &PrePostEnv<'_>,
    ) -> Result<CompatibleChatOutcome, CompatibleChatError> {
        let plan = self.plan(state, env)?;
        let answer = send_chat(client, &plan).await?;
        finish_chat(&plan, answer, state, env)
    }
}
