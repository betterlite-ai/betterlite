use std::sync::{Arc, Mutex, PoisonError};

use betterlite_azure::azure_default_chain::grant::{ClientSecretRecord, UsernamePasswordRecord};
use betterlite_azure::credentials::source::{
    CallableTokenProvider, CredentialSources, SourceFailure,
};
use betterlite_azure::oidc_exchange::OidcExchangeInput;
use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{
    ChatReplyProjection, ChatStreamAssembly, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::openai_base_config::Object;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, RelayCallType,
};
use secrecy::SecretString;

pub(crate) fn base_config(api_base_env: Option<&str>) -> AzurePassthroughConfig {
    AzurePassthroughConfig {
        api_base_env: api_base_env.map(str::to_owned),
        ..Default::default()
    }
}

pub(crate) fn config_with_reply<Reply: Default>(reply: Reply) -> AzurePassthroughConfig<Reply> {
    AzurePassthroughConfig {
        chat_reply: reply,
        ..Default::default()
    }
}

pub(crate) fn config_with_stream<Stream: Default>(
    stream: Stream,
) -> AzurePassthroughConfig<NoChatReplyProjection, Stream> {
    AzurePassthroughConfig {
        chat_stream: stream,
        ..Default::default()
    }
}

pub(crate) fn target<'call>(model: &'call str, endpoint: &'call str) -> PassthroughTarget<'call> {
    PassthroughTarget {
        model,
        custom_llm_provider: "azure",
        endpoint,
    }
}

// keeps the B668 log input beside the reply and hands back the decoded body its projection reads
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
        serde_json::from_slice::<Object>(reply.body).ok()
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

fn failure(message: &str) -> SourceFailure {
    SourceFailure {
        class: String::from("AssertionError"),
        status: None,
        message: String::from(message),
    }
}

pub(crate) struct NoSources;

impl CredentialSources for NoSources {
    fn client_secret<'source>(
        &'source self,
        _record: &ClientSecretRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(failure("no client secret source is configured"))
    }

    fn username_password<'source>(
        &'source self,
        _record: &UsernamePasswordRecord,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(failure("no username and password source is configured"))
    }

    fn sdk_refresh<'source>(
        &'source self,
        _scope: &str,
    ) -> Result<Arc<dyn CallableTokenProvider + 'source>, SourceFailure> {
        Err(failure("no refresh source is configured"))
    }

    fn default_credential<'source>(
        &'source self,
        _scope: &'source str,
    ) -> Option<Arc<dyn CallableTokenProvider + 'source>> {
        None
    }

    fn oidc(&self, _input: &OidcExchangeInput) -> Result<SecretString, SourceFailure> {
        Err(failure("no oidc source is configured"))
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
