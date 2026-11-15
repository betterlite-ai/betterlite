use std::time::Duration;

use secrecy::SecretString;

use super::LITELLM_PROXY_PREFIX;
use super::ProxyOptions;
use super::error::ProxyError;
use crate::chat_choices::endpoint::{
    ModelListRequest, ModelListTransport, get_models as list_models,
};

pub struct ModelsRequest<'request> {
    pub api_base: Option<&'request str>,
    pub api_key: Option<&'request SecretString>,
    // the inherited OpenAI list reads OPENAI_API_KEY when no key is passed
    pub environment_api_key: Option<SecretString>,
    pub deadline: Duration,
}

pub async fn get_models<T>(
    options: &ProxyOptions,
    transport: &T,
    request: &ModelsRequest<'_>,
) -> Result<Vec<String>, ProxyError>
where
    T: ModelListTransport,
{
    let Some(api_base) = options.resolve_api_base(request.api_base) else {
        return Err(ProxyError::MissingApiBase);
    };
    let listed = list_models(
        transport,
        &ModelListRequest {
            api_key: options.resolve_api_key(request.api_key),
            api_base: Some(api_base.as_str()),
            environment_api_key: request.environment_api_key.clone(),
            deadline: request.deadline,
        },
    )
    .await
    .map_err(|source| ProxyError::ModelList { source })?;
    Ok(listed
        .into_iter()
        .map(|model| format!("{LITELLM_PROXY_PREFIX}{model}"))
        .collect())
}
