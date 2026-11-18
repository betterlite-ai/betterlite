use betterlite_openai::watsonx_auth::WatsonxAuth;
use betterlite_openai::watsonx_auth::environment::WatsonxEnvironment;
use betterlite_openai::watsonx_auth::types::WatsonxParams;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::{ExposeSecret as _, SecretString};
use uuid::Uuid;

use crate::data_cloud_dispatch::call::CompletionCall;
use crate::data_cloud_dispatch::context::{DispatchContext, EnvironmentSource};
use crate::watsonx_rerank::WatsonxRerankConfig;

use super::call::convert;
use super::context::{ChatContext, ReplacedCall, handler_call};
use super::error::EnterpriseError;

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

// the popped text: an absent or null key takes the next default, and an explicit text is used as given, empty included
fn pop_text(
    params: &mut WatsonxParams,
    field: &'static str,
) -> Result<Option<String>, EnterpriseError> {
    match params.remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => match value.as_str() {
            Some(text) => Ok(Some(text.to_owned())),
            None => Err(EnterpriseError::FieldType { field }),
        },
    }
}

// an absent or null credentials key takes the other name, and a present object is read as the overrides
fn take_object(
    params: &mut WatsonxParams,
    field: &'static str,
) -> Result<Option<Object>, EnterpriseError> {
    match params.remove(field) {
        None | Some(Json::Null) => Ok(None),
        Some(value) => match serde_json::from_value::<Object>(value) {
            Ok(fields) => Ok(Some(fields)),
            Err(_error) => Err(EnterpriseError::FieldType { field }),
        },
    }
}

// the credentials field: absent or null takes the fallback, and an explicit text is used as given, empty included
fn credential_text(
    fields: &Object,
    field: &'static str,
) -> Result<Option<String>, EnterpriseError> {
    match fields.get(field) {
        None | Some(Json::Null) => Ok(None),
        Some(Json::Text(text)) => Ok(Some(text.clone())),
        Some(Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_)) => {
            Err(EnterpriseError::FieldType { field })
        }
    }
}

// api_key or popped apikey or WATSONX_APIKEY, WATSONX_API_KEY, WX_API_KEY
fn text_api_key(
    base: &DispatchContext<'_>,
    params: &mut WatsonxParams,
) -> Result<Option<SecretString>, EnterpriseError> {
    if let Some(stored) = base.api_key {
        return Ok(Some(SecretString::from(stored.expose_secret().to_owned())));
    }
    if let Some(value) = pop_text(params, "apikey")? {
        return Ok(Some(SecretString::from(value)));
    }
    for name in ["WATSONX_APIKEY", "WATSONX_API_KEY", "WX_API_KEY"] {
        if let Some(value) = base.env.secret(name)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

// api_base or the three pops, else WATSONX_API_BASE, WATSONX_URL, WX_URL, WML_URL
fn text_api_base(
    base: &DispatchContext<'_>,
    params: &mut WatsonxParams,
) -> Result<Option<String>, EnterpriseError> {
    if let Some(value) = base.api_base {
        return Ok(Some(value.to_owned()));
    }
    // the inner pop runs first, so all three leave the params even when url wins
    let base_url = pop_text(params, "base_url")?;
    let api_base = pop_text(params, "api_base")?;
    let url = pop_text(params, "url")?;
    if let Some(value) = url.or(api_base).or(base_url) {
        return Ok(Some(value));
    }
    for name in ["WATSONX_API_BASE", "WATSONX_URL", "WX_URL", "WML_URL"] {
        if let Some(value) = base.env.text(name)? {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

// both credential names are popped, and a present wx_credentials keeps watsonx_credentials from winning
fn text_credentials(
    params: &mut WatsonxParams,
    api_key: &mut Option<SecretString>,
    api_base: &mut Option<String>,
) -> Result<(), EnterpriseError> {
    let nested = take_object(params, "watsonx_credentials")?;
    let wx = take_object(params, "wx_credentials")?;
    let Some(fields) = wx.or(nested) else {
        return Ok(());
    };
    if let Some(value) = credential_text(&fields, "url")? {
        *api_base = Some(value);
    }
    let key = match credential_text(&fields, "apikey")? {
        Some(value) => Some(value),
        None => credential_text(&fields, "api_key")?,
    };
    if let Some(value) = key {
        *api_key = Some(SecretString::from(value));
    }
    let token = match credential_text(&fields, "token")? {
        Some(value) => Some(value),
        None => credential_text(&fields, "watsonx_token")?,
    };
    if let Some(token) = token {
        let stored =
            serde_json::to_value(token).map_err(|source| EnterpriseError::Encode { source })?;
        params.insert(String::from("token"), stored);
    }
    Ok(())
}

// _complete_watsonx_text: the key, base and credentials chains, popping the keys it reads like the dict pops
pub fn text_call(context: &ChatContext<'_>) -> Result<CompletionCall, EnterpriseError> {
    let base = &context.base;
    let mut params: WatsonxParams = convert(base.optional_params)?;
    let mut api_key = text_api_key(base, &mut params)?;
    let mut api_base = text_api_base(base, &mut params)?;
    text_credentials(&mut params, &mut api_key, &mut api_base)?;
    let optional_params: Object = convert(&params)?;
    Ok(handler_call(
        base,
        ReplacedCall {
            optional_params,
            api_key,
            api_base,
            provider: "watsonx_text",
            headers: base.headers.cloned(),
        },
    ))
}

// the watsonx rerank selection returns B5842's config over B5970's auth
#[must_use]
pub const fn rerank_config<T>(auth: &WatsonxAuth<T>) -> WatsonxRerankConfig<'_, T> {
    WatsonxRerankConfig {
        auth,
        new_id: Uuid::new_v4,
    }
}
