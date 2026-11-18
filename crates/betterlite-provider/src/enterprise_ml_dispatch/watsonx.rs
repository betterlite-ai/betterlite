use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_openai::compatible_wire::header_map;
use betterlite_openai::embedding::{EmbeddingInput, EmbeddingResponse};
use betterlite_openai::watsonx_audio::error::body_text;
use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::credentials::get_watsonx_credentials;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_prompts::arguments::json::Object;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use uuid::Uuid;

use crate::data_cloud_dispatch::call::{CompletionCall, EmbeddingCall};
use crate::data_cloud_dispatch::context::EnvironmentSource;
use crate::watsonx_embed::WatsonxEmbeddingConfig;
use crate::watsonx_embed::reply::decode_watsonx_embedding_reply;
use crate::watsonx_rerank::WatsonxRerankConfig;

use super::call::convert;
use super::context::{ChatContext, handler_call};
use super::error::EnterpriseError;
use super::transport::{EnterpriseTransport, SharedTransport};

// every name B5970's WatsonxEnvironment carries, read when the call runs
pub fn environment(source: &EnvironmentSource<'_>) -> Result<WatsonxEnvironment, EnterpriseError> {
    Ok(WatsonxEnvironment {
        watsonx_iam_url: source.text("WATSONX_IAM_URL")?,
        wx_api_key: source.secret("WX_API_KEY")?,
        watsonx_api_key: source.secret("WATSONX_API_KEY")?,
        watsonx_apikey: source.secret("WATSONX_APIKEY")?,
        watsonx_zenapikey: source.secret("WATSONX_ZENAPIKEY")?,
        watsonx_token: source.secret("WATSONX_TOKEN")?,
        watsonx_api_base: source.text("WATSONX_API_BASE")?,
        watsonx_url: source.text("WATSONX_URL")?,
        wx_url: source.text("WX_URL")?,
        wml_url: source.text("WML_URL")?,
        watsonx_project_id: source.text("WATSONX_PROJECT_ID")?,
        wx_project_id: source.text("WX_PROJECT_ID")?,
        project_id: source.text("PROJECT_ID")?,
        watsonx_region: source.text("WATSONX_REGION")?,
        wx_region: source.text("WX_REGION")?,
        region: source.text("REGION")?,
        watsonx_deployment_space_id: source.text("WATSONX_DEPLOYMENT_SPACE_ID")?,
        watsonx_space_id: source.text("WATSONX_SPACE_ID")?,
        wx_space_id: source.text("WX_SPACE_ID")?,
        space_id: source.text("SPACE_ID")?,
    })
}

fn take_truthy(
    params: &mut WatsonxParams,
    field: &'static str,
) -> Result<Option<String>, EnterpriseError> {
    match params.remove(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text).filter(|text| !text.is_empty())),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            Err(EnterpriseError::FieldType { field })
        }
    }
}

// a present null value is an empty pick, which keeps the pop running without falling through to the other name
fn take_object(
    params: &mut WatsonxParams,
    field: &'static str,
) -> Result<Option<serde_json::Map<String, Value>>, EnterpriseError> {
    match params.remove(field) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(serde_json::Map::new())),
        Some(Value::Object(fields)) => Ok(Some(fields)),
        Some(Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_)) => {
            Err(EnterpriseError::FieldType { field })
        }
    }
}

fn credential_text(
    fields: &serde_json::Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, EnterpriseError> {
    match fields.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => {
            Err(EnterpriseError::FieldType { field })
        }
    }
}

// _complete_watsonx_text: the key, base and credentials chains, popping the keys it reads like the dict pops
pub fn text_call(context: &ChatContext<'_>) -> Result<CompletionCall, EnterpriseError> {
    let base = &context.base;
    let mut params: WatsonxParams = convert(base.optional_params)?;
    let mut api_key = base
        .api_key
        .filter(|key| !key.expose_secret().is_empty())
        .map(|key| SecretString::from(key.expose_secret().to_owned()));
    if api_key.is_none() {
        api_key = take_truthy(&mut params, "apikey")?.map(SecretString::from);
    }
    if api_key.is_none() {
        api_key = base.env.secret("WATSONX_APIKEY")?;
    }
    if api_key.is_none() {
        api_key = base.env.secret("WATSONX_API_KEY")?;
    }
    if api_key.is_none() {
        api_key = base.env.secret("WX_API_KEY")?;
    }

    let mut api_base = base
        .api_base
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    if api_base.is_none() {
        // the inner pop runs first, so all three leave the params even when url wins
        let base_url = take_truthy(&mut params, "base_url")?;
        let api_base_parameter = take_truthy(&mut params, "api_base")?;
        let url = take_truthy(&mut params, "url")?;
        api_base = url.or(api_base_parameter).or(base_url);
    }
    if api_base.is_none() {
        api_base = base.env.text("WATSONX_API_BASE")?;
    }
    if api_base.is_none() {
        api_base = base.env.text("WATSONX_URL")?;
    }
    if api_base.is_none() {
        api_base = base.env.text("WX_URL")?;
    }
    if api_base.is_none() {
        api_base = base.env.text("WML_URL")?;
    }

    // both names leave the params whatever the base, and a present wx_credentials keeps watsonx_credentials from winning
    let nested = take_object(&mut params, "watsonx_credentials")?;
    let wx = take_object(&mut params, "wx_credentials")?;
    let mut token = None;
    if let Some(fields) = wx.or(nested) {
        if let Some(value) = credential_text(&fields, "url")? {
            api_base = Some(value);
        }
        if let Some(value) =
            credential_text(&fields, "apikey")?.or(credential_text(&fields, "api_key")?)
        {
            api_key = Some(SecretString::from(value));
        }
        token = credential_text(&fields, "token")?.or(credential_text(&fields, "watsonx_token")?);
    }
    if let Some(token) = token {
        params.insert(String::from("token"), Value::String(token));
    }

    let optional_params: Object = convert(&params)?;
    Ok(handler_call(
        base,
        optional_params,
        api_key,
        api_base,
        "watsonx_text",
        base.headers.cloned(),
    ))
}

// _complete_watsonx hands the context fields over with the literal provider name
pub fn chat_call(context: &ChatContext<'_>) -> CompletionCall {
    super::context::passthrough(&context.base, "watsonx")
}

fn epoch_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

fn source_headers(call: &EmbeddingCall) -> Result<http::HeaderMap, EnterpriseError> {
    match &call.headers {
        None => Ok(http::HeaderMap::new()),
        Some(headers) => header_map(headers).map_err(|source| EnterpriseError::Headers { source }),
    }
}

// the watsonx embedding arm: B5970's credentials, then B5842's config over the shared handler's steps
pub async fn embedding(
    call: &EmbeddingCall,
    source: &EnvironmentSource<'_>,
    transport: &dyn EnterpriseTransport,
) -> Result<EmbeddingResponse, EnterpriseError> {
    let environment = environment(source)?;
    let mut params: WatsonxParams = convert(&call.optional_params)?;
    let credentials = get_watsonx_credentials(
        &mut params,
        call.api_key.as_ref(),
        call.api_base.as_deref(),
        &environment,
    )?;
    if let Some(token) = &credentials.token {
        params.insert(
            String::from("token"),
            Value::String(token.expose_secret().to_owned()),
        );
    }
    let auth = WatsonxAuth::new(
        environment,
        SharedTransport { inner: transport },
        Arc::new(epoch_seconds),
    )?;
    let headers = auth
        .validate_environment(
            source_headers(call)?,
            &mut params,
            Some(&credentials.api_key),
        )
        .await?;
    let config = WatsonxEmbeddingConfig {
        environment: &auth.environment,
    };
    let url = config.get_complete_url(Some(credentials.api_base.as_str()), &mut params)?;
    let input: EmbeddingInput = convert(&call.input)?;
    let body = config.transform_embedding_request(&call.model, &input, params)?;
    let bytes = serde_json::to_vec(&body).map_err(|source| EnterpriseError::Encode { source })?;
    let request = super::transport::request(Method::POST, &url, &headers, bytes, call.timeout)?;
    let response = transport
        .send(request)
        .await
        .map_err(|source| EnterpriseError::Transport { source })?;
    if !(200..300).contains(&response.status) {
        return Err(betterlite_openai::watsonx_auth::error::WatsonxError::Api {
            status_code: response.status,
            message: body_text(&response.body),
            headers: Some(response.headers),
        }
        .into());
    }
    let reply = decode_watsonx_embedding_reply(&response.body)?;
    Ok(WatsonxEmbeddingConfig::transform_embedding_response(
        &call.model,
        reply,
    ))
}

// the watsonx rerank selection returns B5842's config over B5970's auth
pub fn rerank_config<T>(auth: &WatsonxAuth<T>) -> WatsonxRerankConfig<'_, T> {
    WatsonxRerankConfig {
        auth,
        new_id: Uuid::new_v4,
    }
}
