use std::sync::Arc;

use betterlite_config::sections::azure_credentials::DeploymentSettings;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::decisions::Json;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::compatible_wire::LogInput;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayedResponse,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, logged_relay_shape, model_group_from, replace_path_segment,
    strip_leading_model_segment,
};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use url::Url;

use crate::azure_urls::{
    EnvironmentHeadersRequest, get_base_azure_url, validate_azure_environment,
};
use crate::credentials::source::{CallableTokenProvider, CredentialSources};
use crate::passthrough::error::AzurePassthroughError;
use crate::passthrough::logging::{
    AzureRelayedLog, ChatReplyProjection, ChatStreamAssembly, NoChatReplyProjection,
    NoChatStreamAssembly, OPENAI_RELAY_SHAPES, RESPONSES_SUFFIX, logged_responses_stream,
    relayed_messages,
};
use crate::passthrough::url::{
    is_filled, params_api_version, request_api_version, without_api_version,
};

const CHAT_COMPLETIONS: &str = "chat/completions";
const STREAM: &str = "stream";
const NO_MESSAGE_CONTENT: &str = "no-message-pass-through-endpoint";

// the caller resolves AZURE_API_BASE and AZURE_API_KEY, and an empty value counts as unset
#[derive(Debug, Default)]
pub struct AzurePassthroughConfig<Reply = NoChatReplyProjection, Stream = NoChatStreamAssembly> {
    pub api_base_env: Option<String>,
    pub api_key_env: Option<SecretString>,
    pub model_call_details: Option<Object>,
    pub chat_reply: Reply,
    pub chat_stream: Stream,
}

// configured_key is litellm.api_key, litellm.azure_key, AZURE_OPENAI_API_KEY and AZURE_API_KEY, resolved by the caller
pub struct AzurePassthroughEnvironment<'data> {
    pub headers: Headers,
    pub api_key: Option<&'data SecretString>,
    // litellm_params.api_key, kept to show the positional key overwrites it
    pub params_key: Option<&'data SecretString>,
    pub configured_key: Option<&'data SecretString>,
    pub credentials: &'data DeploymentSettings,
    pub refresh_enabled: bool,
    pub provider: Option<Arc<dyn CallableTokenProvider + 'data>>,
    pub sources: &'data dyn CredentialSources,
}

pub fn validate_environment(
    request: AzurePassthroughEnvironment<'_>,
) -> Result<Headers, AzurePassthroughError> {
    // {**litellm_params, "api_key": api_key} replaces litellm_params.api_key even when the positional key is None
    let positional = request.api_key;
    let key = positional
        .filter(|key| !key.expose_secret().is_empty())
        .or(request.configured_key);
    validate_azure_environment(EnvironmentHeadersRequest {
        headers: request.headers,
        api_key: key,
        credentials: request.credentials,
        refresh_enabled: request.refresh_enabled,
        provider: request.provider,
        sources: request.sources,
    })
    .map_err(AzurePassthroughError::from)
}

fn passthrough_log_input(request_data: &Object) -> LogInput {
    LogInput {
        messages: Json::List(vec![Json::Object(IndexMap::from([
            (String::from("role"), Json::Text(String::from("user"))),
            (
                String::from("content"),
                Json::Text(String::from(NO_MESSAGE_CONTENT)),
            ),
        ]))]),
        data: RawBody::Object(passthrough_request_body(request_data)),
    }
}

fn passthrough_request_body(request_data: &Object) -> IndexMap<String, Json> {
    request_data
        .iter()
        .filter_map(|(name, value)| {
            serde_json::from_value::<Json>(value.clone())
                .ok()
                .map(|json| (name.clone(), json))
        })
        .collect()
}

// LiteLLM's get_base_model returns its argument and get_models defers to the model-info owner, so neither has an override here.
impl<Reply, Stream> AzurePassthroughConfig<Reply, Stream> {
    #[must_use]
    pub fn get_api_base<'base>(&'base self, api_base: Option<&'base str>) -> Option<&'base str> {
        api_base
            .filter(|base| !base.is_empty())
            .or_else(|| self.api_base_env.as_deref().filter(|base| !base.is_empty()))
    }

    #[must_use]
    pub fn get_api_key<'key>(
        &'key self,
        api_key: Option<&'key SecretString>,
    ) -> Option<&'key SecretString> {
        api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| {
                self.api_key_env
                    .as_ref()
                    .filter(|key| !key.expose_secret().is_empty())
            })
    }

    pub fn logging_non_streaming_response(
        &self,
        target: &PassthroughTarget<'_>,
        response: &RelayedResponse<'_>,
        headers: &Headers,
        request_data: &Object,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<AzureRelayedLog>
    where
        Reply: ChatReplyProjection,
    {
        if !target.endpoint.contains(CHAT_COMPLETIONS) {
            let relayed = logged_relay_shape(
                &OPENAI_RELAY_SHAPES,
                response.status_code,
                response.body,
                logging_obj,
                target.endpoint,
            )?;
            return Some(AzureRelayedLog::Relay(relayed));
        }
        let reply = ResponseReply {
            status: response.status_code,
            headers,
            body: response.body,
        };
        self.chat_reply
            .project_chat_reply(target.model, &reply, passthrough_log_input(request_data))
            .map(AzureRelayedLog::Chat)
    }
}

impl<Reply, Stream> BasePassthroughConfig for AzurePassthroughConfig<Reply, Stream>
where
    Reply: ChatReplyProjection,
    Stream: ChatStreamAssembly,
{
    type Error = AzurePassthroughError;
    type Logged = AzureRelayedLog;

    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        request_data
            .get(STREAM)
            .and_then(|value| serde_json::from_value::<Json>(value.clone()).ok())
            .as_ref()
            .is_some_and(is_filled)
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let Some(base) = self.get_api_base(request.api_base) else {
            return Err(AzurePassthroughError::MissingApiBase);
        };
        let model_group = model_group_from(request.litellm_params);
        let routed = if model_group.is_empty() {
            request.target.endpoint.to_owned()
        } else {
            replace_path_segment(request.target.endpoint, &model_group, request.target.model)
        };
        let native_endpoint =
            strip_leading_model_segment(&routed, &[request.target.model.to_owned()]);
        let caller_version = request_api_version(request.request_query_params)?;
        let relay_base = if caller_version.is_some() {
            without_api_version(base)?
        } else {
            base.to_owned()
        };
        // a caller version short-circuits LiteLLM's `caller or litellm_params.get(...)`, so a badly typed deployment version is never read
        let mut version = caller_version;
        if version.is_none() {
            version = params_api_version(request.litellm_params)?;
        }
        let complete = get_base_azure_url(
            Some(&relay_base),
            version.as_deref(),
            None,
            &native_endpoint,
        )?;
        let url = Url::parse(&complete).map_err(|source| PassthroughError::InvalidUrl {
            url: complete.clone(),
            source,
        })?;
        Ok((url, base.to_owned()))
    }

    fn logging_non_streaming_response(
        &self,
        target: &PassthroughTarget<'_>,
        response: &RelayedResponse<'_>,
        request_data: &Object,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Self::Logged> {
        // the trait cannot see the reply headers, so a caller holding them calls the inherent method
        self.logging_non_streaming_response(
            target,
            response,
            &Headers::new(),
            request_data,
            logging_obj,
        )
    }

    fn handle_collected_chunks(
        &self,
        all_chunks: &[String],
        logging_obj: &mut dyn PassthroughLogging,
        target: &PassthroughTarget<'_>,
    ) -> Option<Self::Logged> {
        let relayed_path = format!("/{}", target.endpoint.trim_matches('/'));
        if relayed_path.ends_with(RESPONSES_SUFFIX) {
            let terminal_event = logged_responses_stream(all_chunks, logging_obj)?;
            return Some(AzureRelayedLog::Terminal(Box::new(terminal_event)));
        }
        if !target.endpoint.contains(CHAT_COMPLETIONS) {
            return None;
        }
        let messages = self.model_call_details.as_ref().and_then(relayed_messages);
        self.chat_stream
            .complete_streaming_response(all_chunks, target.model, messages.as_deref())
            .map(AzureRelayedLog::Chat)
    }
}
