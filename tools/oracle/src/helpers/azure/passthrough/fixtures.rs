use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{
    ChatReplyProjection, ChatStreamAssembly, NoChatReplyProjection,
};
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
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, RelayCallType,
};
use betterlite_redact::secret::{MINIMUM_CUSTOM_KEY_LENGTH, Redactor};
use chrono::DateTime;
use secrecy::SecretString;

const INIT: &str = r#"{"model":"gpt-4o","call_type":"completion","start_time":"2026-01-01T00:00:00+00:00","litellm_call_id":"b548","function_id":"b548","litellm_trace_id":"trace-b548"}"#;
const NOW: &str = "2026-01-01T00:05:00+00:00";

pub(crate) fn base_config(api_base_env: Option<&str>) -> AzurePassthroughConfig {
    AzurePassthroughConfig {
        api_base_env: api_base_env.map(str::to_owned),
        ..AzurePassthroughConfig::default()
    }
}

pub(crate) fn config_with_reply<Reply: Default>(reply: Reply) -> AzurePassthroughConfig<Reply> {
    AzurePassthroughConfig {
        chat_reply: reply,
        ..AzurePassthroughConfig::default()
    }
}

pub(crate) fn config_with_stream<Stream: Default>(
    stream: Stream,
) -> AzurePassthroughConfig<NoChatReplyProjection, Stream> {
    AzurePassthroughConfig {
        chat_stream: stream,
        ..AzurePassthroughConfig::default()
    }
}

pub(crate) fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure",
        endpoint,
    }
}

pub(crate) fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

#[derive(Debug, Default)]
pub(crate) struct Silent;

impl LogSink for Silent {
    fn emit(&self, _record: LogRecord) {}
}

// the real post-call sink B668 writes through, built on a stable clock and call id
struct LoggingFixture {
    redactor: Redactor,
    state: LoggingState,
    sink: Silent,
}

impl LoggingFixture {
    fn new() -> Option<Self> {
        let redactor = Redactor::new(MINIMUM_CUSTOM_KEY_LENGTH).ok()?;
        let state = LoggingState::new(
            serde_json::from_str::<LoggingInit>(INIT).ok()?,
            CorrelationSetup {
                context: &mut CorrelationContext::default(),
                settings: CorrelationSettings::default(),
            },
            &redactor,
            || String::from("trace"),
            Duration::default,
        )
        .ok()?;
        Some(Self {
            redactor,
            state,
            sink: Silent,
        })
    }

    fn project(&mut self, reply: &ResponseReply<'_>, input: LogInput) -> Option<Object> {
        let env = PrePostEnv {
            debug: DebugSettings::default(),
            globals: GlobalFlags::default(),
            decision: Decision {
                content: false,
                key_info: false,
            },
            redactor: &self.redactor,
            log: &self.sink,
            input_callbacks: &[],
            sentry: None,
            logger_fn: None,
            now: DateTime::parse_from_rfc3339(NOW).ok()?,
        };
        match transform_response(
            reply,
            &mut self.state,
            &env,
            input,
            |decoded, _raw_headers, _hidden_headers| Ok(decoded),
        ) {
            Ok(value) => value.as_object().cloned(),
            Err(_failure) => None,
        }
    }
}

// keeps the B668 projection's log input alongside the projected reply
#[derive(Debug, Default)]
pub(crate) struct OracleChatProjection {
    logged: Mutex<Option<(Json, Json)>>,
}

impl OracleChatProjection {
    pub(crate) fn logged(&self) -> Option<(Json, Json)> {
        self.logged
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ChatReplyProjection for OracleChatProjection {
    fn project_chat_reply(
        &self,
        _model: &str,
        reply: &ResponseReply<'_>,
        input: LogInput,
    ) -> Option<Object> {
        let recorded = (input.messages.clone(), Json::from(&input.data));
        *self.logged.lock().unwrap_or_else(PoisonError::into_inner) = Some(recorded);
        LoggingFixture::new()?.project(reply, input)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RecordedAssembly {
    pub(crate) model: String,
    pub(crate) messages: Option<Vec<Object>>,
}

#[derive(Debug, Default)]
pub(crate) struct RecordingStream {
    assembly: Mutex<Option<RecordedAssembly>>,
}

impl RecordingStream {
    pub(crate) fn assembly(&self) -> Option<RecordedAssembly> {
        self.assembly
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl ChatStreamAssembly for RecordingStream {
    fn complete_streaming_response(
        &self,
        _chunks: &[String],
        model: &str,
        messages: Option<&[Object]>,
    ) -> Option<Object> {
        let recorded = RecordedAssembly {
            model: model.to_owned(),
            messages: messages.map(<[Object]>::to_vec),
        };
        *self.assembly.lock().unwrap_or_else(PoisonError::into_inner) = Some(recorded);
        Some(Object::new())
    }
}

#[derive(Debug, Default)]
pub(crate) struct RecordingLogging {
    pub(crate) call_type: Option<RelayCallType>,
}

impl PassthroughLogging for RecordingLogging {
    fn set_call_type(&mut self, call_type: RelayCallType) {
        self.call_type = Some(call_type);
    }
}

pub(crate) struct NoSources;

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

pub(crate) fn deployment() -> betterlite_config::sections::azure_credentials::DeploymentSettings {
    betterlite_config::sections::azure_credentials::DeploymentSettings {
        azure_ad_token: None,
        tenant_id: None,
        client_id: None,
        client_secret: None,
        azure_scope: String::from("https://cognitiveservices.azure.com/.default"),
        azure_username: None,
        azure_password: None,
    }
}
