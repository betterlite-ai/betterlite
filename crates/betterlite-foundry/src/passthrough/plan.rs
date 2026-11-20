use std::fmt::{Debug, Formatter, Result as FormatResult};
use std::sync::Arc;

use betterlite_azure::passthrough::config::AzurePassthroughConfig;
use betterlite_azure::passthrough::logging::{
    ChatReplyProjection, ChatStreamAssembly, NoChatReplyProjection, NoChatStreamAssembly,
};
use betterlite_logging::passthrough_logging_state::response::PassThroughResponseObject;
use betterlite_models::decisions::Json as DecisionsJson;
use betterlite_ocr::response::OCRResponse;
use betterlite_openai::chat_choices::reply::ResponseReply;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::types::{
    PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayedBody, RelayedResponse,
};
use betterlite_openai::passthrough::transformation::{
    BasePassthroughConfig, logged_relay_shape, model_group_from, relayed_body,
    strip_leading_model_segment,
};
use betterlite_prompts::arguments::json::Json;
use secrecy::SecretString;
use url::Url;

use crate::credentials::model_info::ModelInfoAccessors;

use super::error::FoundryPassthroughError;
use super::logging::{
    FOUNDRY_RELAY_SHAPES, FoundryPassthroughLogging, FoundryRelayedLog, TraitFoundryLogging,
};
use super::ocr::{NativeOcrPassthrough, decode_logged_ocr_reply};
use super::url::{foundry_root, relay_query_params, without_repeated_native_prefix};

const STREAM: &str = "stream";
// litellm_params carries the deployment version under its own underscore key
const PARAMS_API_VERSION: &str = "api_version";

// one relayed reply with the route it came from: LiteLLM's logging methods read these beside each other
pub struct FoundryLogInput<'data> {
    pub target: &'data PassthroughTarget<'data>,
    pub reply: &'data ResponseReply<'data>,
    pub request_data: &'data Object,
    pub request_url: &'data str,
}

// AzureAIPassthroughConfig: the Azure projection, the native OCR binding and the deployment params in one plan
pub struct FoundryPassthroughPlan<Reply = NoChatReplyProjection, Stream = NoChatStreamAssembly> {
    pub global_api_base: Option<String>,
    pub global_api_key: Option<SecretString>,
    // AZURE_AI_API_BASE and AZURE_AI_API_KEY, resolved by the caller
    pub azure: AzurePassthroughConfig<Reply, Stream>,
    pub ocr: Option<Arc<dyn NativeOcrPassthrough>>,
    // LiteLLM reads litellm_metadata.model_group from here; the map also holds api_key and Entra secrets
    pub litellm_params: Object,
}

impl<Reply, Stream> Debug for FoundryPassthroughPlan<Reply, Stream> {
    fn fmt(&self, f: &mut Formatter<'_>) -> FormatResult {
        f.debug_struct("FoundryPassthroughPlan")
            .field("global_api_base", &self.global_api_base)
            .field("environment_api_base", &self.azure.api_base_env)
            .field(
                "global_api_key",
                &self.global_api_key.as_ref().map(|_key| "<redacted>"),
            )
            .field(
                "environment_api_key",
                &self.azure.api_key_env.as_ref().map(|_key| "<redacted>"),
            )
            .field("model_group", &model_group_from(&self.litellm_params))
            .field("injected_ocr", &self.ocr.is_some())
            .finish_non_exhaustive()
    }
}

impl<Reply, Stream> FoundryPassthroughPlan<Reply, Stream> {
    #[must_use]
    pub fn accessors(&self) -> ModelInfoAccessors<'_> {
        ModelInfoAccessors {
            global_api_base: self.global_api_base.as_deref(),
            global_api_key: self.global_api_key.as_ref(),
            environment_api_base: self.azure.api_base_env.as_deref(),
            environment_api_key: self.azure.api_key_env.as_ref(),
        }
    }

    // logged_ocr_response: a 200 whose body the OCR projector decodes sets call_type=aocr, and a failure warns
    pub fn logged_ocr_response(
        &self,
        relayed: &FoundryLogInput<'_>,
        logging_obj: &mut dyn FoundryPassthroughLogging,
    ) -> Option<OCRResponse> {
        if relayed.reply.status != 200 {
            return None;
        }
        let group = model_group_from(&self.litellm_params);
        let native_endpoint = strip_leading_model_segment(
            relayed.target.endpoint,
            &[relayed.target.model.to_owned(), group],
        );
        match decode_logged_ocr_reply(
            self.ocr.as_deref(),
            relayed.target.model,
            &native_endpoint,
            relayed.reply.body,
        ) {
            Ok(Some(response)) => {
                logging_obj.set_ocr_call_type();
                Some(response)
            }
            Ok(None) => None,
            Err(error) => {
                tracing::warn!(
                    "azure_ai passthrough: OCR body from {} is not costable: {}",
                    relayed.request_url,
                    error
                );
                None
            }
        }
    }

    // logging_non_streaming_response: the Azure projection first, then OCR, the Foundry relay shapes and the relayed body
    pub fn logging_relayed_response(
        &self,
        relayed: &FoundryLogInput<'_>,
        logging_obj: &mut dyn FoundryPassthroughLogging,
    ) -> Option<FoundryRelayedLog>
    where
        Reply: ChatReplyProjection,
    {
        let azure = {
            let trait_logging: &mut dyn PassthroughLogging = &mut *logging_obj;
            self.azure.logging_reply_with_headers(
                relayed.target,
                relayed.reply,
                relayed.request_data,
                trait_logging,
            )
        };
        if let Some(azure_log) = azure {
            return Some(FoundryRelayedLog::Azure(azure_log));
        }
        if let Some(ocr) = self.logged_ocr_response(relayed, logging_obj) {
            return Some(FoundryRelayedLog::Ocr(Box::new(ocr)));
        }
        let shaped = {
            let trait_logging: &mut dyn PassthroughLogging = &mut *logging_obj;
            logged_relay_shape(
                &FOUNDRY_RELAY_SHAPES,
                relayed.reply.status,
                relayed.reply.body,
                trait_logging,
                relayed.target.endpoint,
            )
        };
        if let Some(shaped_log) = shaped {
            return Some(FoundryRelayedLog::Relay(shaped_log));
        }
        standard_response(relayed.reply.body).map(FoundryRelayedLog::Standard)
    }
}

impl<Reply, Stream> BasePassthroughConfig for FoundryPassthroughPlan<Reply, Stream>
where
    Reply: ChatReplyProjection,
    Stream: ChatStreamAssembly,
{
    type Error = FoundryPassthroughError;
    type Logged = FoundryRelayedLog;

    // is_streaming_request: bool(request_data.get("stream")) for every endpoint
    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        request_data
            .get(STREAM)
            .and_then(|value| serde_json::from_value::<Json>(value.clone()).ok())
            .is_some_and(|value| value.is_truthy())
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let Some(base) = self.accessors().api_base(request.api_base) else {
            return Err(FoundryPassthroughError::MissingApiBase);
        };
        let model_group = model_group_from(request.litellm_params);
        let native_endpoint = strip_leading_model_segment(
            request.target.endpoint,
            &[request.target.model.to_owned(), model_group],
        );
        let root = without_repeated_native_prefix(&foundry_root(&base)?, &native_endpoint)?;
        // api_version_from: TypeAdapter(str | None) accepts only a string or null
        let api_version = request
            .litellm_params
            .get(PARAMS_API_VERSION)
            .and_then(serde_json::Value::as_str);
        let query_params = relay_query_params(request.request_query_params, api_version, &base)?;
        let url = self.format_url(&native_endpoint, &root, query_params.as_ref())?;
        Ok((url, root))
    }

    fn logging_non_streaming_response(
        &self,
        target: &PassthroughTarget<'_>,
        response: &RelayedResponse<'_>,
        request_data: &Object,
        logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Self::Logged> {
        // the trait carries neither the reply headers nor the request URL, so the OCR warning names the endpoint
        let headers = Headers::new();
        let reply = ResponseReply {
            status: response.status_code,
            headers: &headers,
            body: response.body,
        };
        let relayed = FoundryLogInput {
            target,
            reply: &reply,
            request_data,
            request_url: target.endpoint,
        };
        let mut foundry_logging = TraitFoundryLogging(logging_obj);
        self.logging_relayed_response(&relayed, &mut foundry_logging)
    }

    // handle_logging_collected_chunks: the Azure assembly is used unchanged
    fn handle_collected_chunks(
        &self,
        all_chunks: &[String],
        logging_obj: &mut dyn PassthroughLogging,
        target: &PassthroughTarget<'_>,
    ) -> Option<Self::Logged> {
        self.azure
            .handle_collected_chunks(all_chunks, logging_obj, target)
            .map(FoundryRelayedLog::Azure)
    }
}

// StandardPassThroughResponseObject(response=relayed_body(httpx_response))
fn standard_response(body: &[u8]) -> Option<PassThroughResponseObject> {
    let response: DecisionsJson = match relayed_body(body).ok()? {
        RelayedBody::Json(object) => serde_json::from_value(serde_json::json!(object)).ok()?,
        RelayedBody::Text(text) => DecisionsJson::Text(text),
    };
    Some(PassThroughResponseObject { response })
}
