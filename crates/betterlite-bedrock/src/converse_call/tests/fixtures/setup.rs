use super::bedrock::agentcore_stream::IdSource;
use super::bedrock::converse_call::{ConverseCall, ConverseEnvironment};
use super::bedrock::converse_request::{ConverseMessageSettings, ConverseSettings};
use super::bedrock::converse_response::wire::ModelResponse;
use super::input::Input;
use super::run::FixtureError;
use betterlite_config::proxy::ProxyPolicy;
use betterlite_config::tls::Verification;
use betterlite_http::proxy::{ProxiedClient, base_builder};
use betterlite_http::user_url::fetch::UserUrlFetcher;
use betterlite_http::user_url::validate::{Resolve, UrlValidator};
use betterlite_logging::logging_pre_post::env::{LogRecord, LogSink};
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_prompts::bedrock_images::remote::{RemoteImage, TRANSPORT_LIMITS};
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

pub(super) struct Quiet;
impl LogSink for Quiet {
    fn emit(&self, _record: LogRecord) {}
}
impl IdSource for Quiet {
    fn next_id(&self) -> Uuid {
        Uuid::nil()
    }
}

#[async_trait::async_trait]
impl Resolve for Quiet {
    async fn resolve(&self, _host: &str, _port: u16) -> io::Result<Vec<SocketAddr>> {
        Err(io::Error::other("fixture forbids network"))
    }
}

pub(super) fn remote() -> Result<RemoteImage<Quiet>, FixtureError> {
    let policy = Arc::new(ProxyPolicy {
        no_proxy: String::new(),
        primary: BTreeMap::new(),
        fallback: BTreeMap::new(),
        warnings: Vec::new(),
    });
    Ok(RemoteImage {
        client: ProxiedClient::build(base_builder(), &policy, TRANSPORT_LIMITS)?,
        urls: UserUrlFetcher {
            validator: UrlValidator::new(Quiet, &[], &Verification::Enabled),
            validation: true,
        },
        deadline: TRANSPORT_LIMITS.read_timeout,
        max_download_size_mb: 50.0,
    })
}

pub(super) fn state(redactor: &Redactor) -> Result<LoggingState, FixtureError> {
    let init: LoggingInit = serde_json::from_str(
        r#"{"model":"anthropic.claude-3-sonnet-20240229-v1:0","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#,
    )?;
    Ok(LoggingState::new(
        init,
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || "fixture".to_owned(),
        Duration::default,
    )?)
}

pub(super) fn call<'input>(
    input: &'input Input,
    bearer: Option<&'input SecretString>,
    headers: &'input [(String, String)],
) -> ConverseCall<'input> {
    ConverseCall {
        model: &input.model,
        messages: &[],
        optional_params: input.params.clone(),
        litellm_params: IndexMap::default(),
        extra_headers: Some(headers),
        api_base: Some("https://bedrock-runtime.us-west-2.amazonaws.com"),
        api_key: bearer,
        timeout: Some(Duration::from_secs(3)),
        acompletion: input.asynchronous,
        control: ControlOptions {
            stream_chunk_size: input.native.chunk_size,
        },
        skip_pre_call_logging: input.native.skip_log,
        model_response: ModelResponse {
            id: "fixture".to_owned(),
            created: 0,
            model: input.model.clone(),
            object: "chat.completion".to_owned(),
            system_fingerprint: None,
            choices: Vec::new(),
            usage: None,
            service_tier: None,
            trace: None,
        },
        settings: ConverseSettings {
            global_drop: false,
            messages: ConverseMessageSettings {
                modify_params: false,
                neutralize_orphans: true,
            },
            request_metadata: None,
        },
        environment: ConverseEnvironment::default(),
        now: SystemTime::UNIX_EPOCH,
    }
}

impl super::bedrock::converse_usage::transform::ReasoningTokenCounter for Quiet {
    type Failure = Infallible;
    fn count_reasoning_tokens(&self, _text: &str) -> Result<i64, Self::Failure> {
        Ok(2)
    }
}
