use http::HeaderMap;
use secrecy::SecretString;
use serde_json::Value;

use super::error::WatsonxChatError;
use super::urls::get_complete_url;
use crate::watsonx_auth::WatsonxAuth;
use crate::watsonx_auth::credentials::get_api_params;
use crate::watsonx_auth::payload::prepare_payload;
use crate::watsonx_auth::transport::WatsonxTransport;
use crate::watsonx_auth::types::WatsonxParams;

pub struct WatsonxChatRequest<'request> {
    pub model: &'request str,
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    pub headers: HeaderMap,
    pub options: WatsonxParams,
}

// what WatsonXChatHandler.completion hands to OpenAILikeChatHandler.completion, which also gets custom_endpoint true, the same api_key and no fake_stream
#[derive(Debug)]
pub struct WatsonxChatPlan {
    pub model: String,
    pub url: String,
    pub headers: HeaderMap,
    pub options: WatsonxParams,
}

impl WatsonxChatPlan {
    pub async fn prepare<T: WatsonxTransport>(
        auth: &WatsonxAuth<T>,
        request: WatsonxChatRequest<'_>,
    ) -> Result<Self, WatsonxChatError> {
        let WatsonxChatRequest {
            model,
            api_base,
            api_key,
            headers: caller_headers,
            mut options,
        } = request;
        let api_params = get_api_params(&mut options, Some(model), &auth.environment)?;
        let headers = auth
            .validate_environment(caller_headers, &mut options, api_key)
            .await?;
        let payload = prepare_payload(model, &api_params);
        let payload_model = payload
            .get("model_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        options.extend(payload);
        let stream = match options.get("stream") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(flag)) => *flag,
            Some(Value::Number(_) | Value::String(_) | Value::Array(_) | Value::Object(_)) => {
                return Err(WatsonxChatError::InvalidField { field: "stream" });
            }
        };
        let url = get_complete_url(api_base, model, stream, &mut options, &auth.environment)?;
        Ok(Self {
            model: payload_model,
            url,
            headers,
            options,
        })
    }
}
