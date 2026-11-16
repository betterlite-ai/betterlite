use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_logging::logging_pre_post::env::{
    DebugSettings, GlobalFlags, LogRecord, LogSink, PrePostEnv,
};
use betterlite_logging::logging_privacy::request::Decision;
use betterlite_logging::logging_state::correlation::{
    CorrelationContext, CorrelationSettings, CorrelationSetup,
};
use betterlite_logging::logging_state::{LoggingInit, LoggingState};
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::reply::{ResponseReply, transform_response};
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayCallType,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use crate::credentials::source::{
    CallableTokenProvider, CredentialSources, ProviderFailure, SourceFailure, TokenReply,
};
use crate::oidc_exchange::OidcExchangeInput;
use crate::passthrough::config::AzurePassthroughConfig;
use crate::passthrough::error::AzurePassthroughError;
use crate::passthrough::logging::{ChatReplyProjection, ChatStreamAssembly};

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"call-1","function_id":"fn-1","litellm_trace_id":"trace-1"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

pub struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

pub fn logging_state(redactor: &Redactor) -> (LoggingState, PrePostEnv<'_>) {
    let state = LoggingState::new(
        serde_json::from_str::<LoggingInit>(INIT).unwrap(),
        CorrelationSetup {
            context: &mut CorrelationContext::default(),
            settings: CorrelationSettings::default(),
        },
        redactor,
        || String::from("trace"),
        Duration::default,
    )
    .unwrap();
    let env = PrePostEnv {
        debug: DebugSettings::default(),
        globals: GlobalFlags::default(),
        decision: Decision {
            content: false,
            key_info: false,
        },
        redactor,
        log: &Silent,
        input_callbacks: &[],
        sentry: None,
        logger_fn: None,
        now: DateTime::parse_from_rfc3339(NOW).unwrap(),
    };
    (state, env)
}

pub fn object(source: &str) -> Object {
    serde_json::from_str(source).unwrap()
}

pub fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

pub fn deployment() -> DeploymentSettings {
    DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}

pub fn base_config(api_base_env: Option<&str>) -> AzurePassthroughConfig {
    AzurePassthroughConfig {
        api_base_env: api_base_env.map(str::to_owned),
        ..AzurePassthroughConfig::default()
    }
}

pub fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure",
        endpoint,
    }
}

pub fn complete(
    config: &AzurePassthroughConfig,
    api_base: Option<&str>,
    target: PassthroughTarget<'_>,
    request_query_params: Option<&Object>,
    litellm_params: &Object,
) -> Result<(Url, String), AzurePassthroughError> {
    config.get_complete_url(&PassthroughUrlRequest {
        api_base,
        api_key: None,
        target,
        request_query_params,
        litellm_params,
    })
}

pub fn terminal_chunks(id: &str) -> Vec<String> {
    vec![format!(
        r#"data: {{"type":"response.completed","sequence_number":2,"response":{{"id":"{id}","created_at":7,"output":[],"status":"completed","x-extra":{{"kept":true}}}}}}"#
    )]
}

pub fn chat_chunks() -> Vec<String> {
    vec![
        String::from(
            r#"data: {"id":"chatcmpl-1","object":"chat.completion.chunk","created":7,"model":"gpt-4o","choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#,
        ),
        String::from("data: [DONE]"),
    ]
}

#[derive(Debug, Default)]
pub struct RecordingLogging {
    pub call_type: Option<RelayCallType>,
}

impl PassthroughLogging for RecordingLogging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type);
    }
}

// runs the real B668 projection with a passthrough projection that keeps the decoded reply
#[derive(Debug, Default)]
pub struct RealChatProjection {
    pub models: Mutex<Vec<String>>,
    pub inputs: Mutex<Vec<(Json, RawBody)>>,
}

impl ChatReplyProjection for RealChatProjection {
    fn project_chat_reply(
        &self,
        model: &str,
        reply: &ResponseReply<'_>,
        input: LogInput,
    ) -> Option<Object> {
        self.models
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model.to_owned());
        self.inputs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((input.messages.clone(), input.data.clone()));
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).unwrap();
        let (mut state, env) = logging_state(&redactor);
        let projected = transform_response(
            reply,
            &mut state,
            &env,
            input,
            |decoded, _response_headers, _hidden_headers| Ok(decoded),
        );
        match projected {
            Ok(value) => value.as_object().cloned(),
            Err(_failure) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedAssembly {
    pub model: String,
    pub chunks: Vec<String>,
    pub messages: Option<Vec<Object>>,
}

#[derive(Debug, Default)]
pub struct RecordingStream {
    pub recorded: Mutex<Option<RecordedAssembly>>,
    pub answer: Option<Object>,
}

impl RecordingStream {
    pub fn taken(&self) -> Option<RecordedAssembly> {
        self.recorded
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ChatStreamAssembly for RecordingStream {
    fn complete_streaming_response(
        &self,
        chunks: &[String],
        model: &str,
        messages: Option<&[Object]>,
    ) -> Option<Object> {
        let recorded = RecordedAssembly {
            model: model.to_owned(),
            chunks: chunks.to_vec(),
            messages: messages.map(<[Object]>::to_vec),
        };
        *self.recorded.lock().unwrap_or_else(PoisonError::into_inner) = Some(recorded);
        self.answer.clone()
    }
}

pub struct NoSources;

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("no client secret source is configured"),
        })
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("no username and password source is configured"),
        })
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("no refresh source is configured"),
        })
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("no oidc source is configured"),
        })
    }
}

pub struct FixedProvider {
    pub token: SecretString,
}

impl CallableTokenProvider for FixedProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Ok(TokenReply::Text(self.token.clone()))
    }
}

pub struct FailingProvider {
    pub message: String,
}

impl CallableTokenProvider for FailingProvider {
    fn token(&self) -> Result<TokenReply, ProviderFailure> {
        Err(ProviderFailure {
            class: String::from("RuntimeError"),
            message: self.message.clone(),
        })
    }
}

pub struct RecordingSources {
    pub label: &'static str,
    pub calls: Mutex<Vec<String>>,
}

impl RecordingSources {
    pub fn new(label: &'static str) -> Self {
        Self {
            label,
            calls: Mutex::new(Vec::new()),
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn provider(&self, scope: &str) -> Arc<dyn CallableTokenProvider + '_> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(format!("{}:{scope}", self.label));
        Arc::new(FixedProvider {
            token: secret(&format!("{}:{scope}", self.label)),
        })
    }
}

impl CredentialSources for RecordingSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("the recording fixture serves only the SDK refresh"),
        })
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("the recording fixture serves only the SDK refresh"),
        })
    }

    fn sdk_refresh<'source>(
        &'source self,
        scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Ok(self.provider(scope))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(SourceFailure {
            class: String::from("AssertionError"),
            status: None,
            message: String::from("the recording fixture serves only the SDK refresh"),
        })
    }
}

pub fn provider_token(provider: &(dyn CallableTokenProvider + '_)) -> String {
    match provider.token() {
        Ok(TokenReply::Text(token)) => token.expose_secret().to_owned(),
        Ok(TokenReply::NonText { type_name }) => type_name,
        Err(failure) => failure.message,
    }
}

pub fn headers(values: &[(&str, &str)]) -> Headers {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<String, String>>()
}
