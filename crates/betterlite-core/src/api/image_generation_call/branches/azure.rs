use betterlite_azure::credentials::source::CallableTokenProvider;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use secrecy::{ExposeSecret as _, SecretString};
use std::sync::Arc;

use super::super::call::ImageGenerationCall;
use super::super::error::{ImageGenerationException, raise};
use super::super::operations::{
    AzureRequest, Branch, BranchAnswer, EntraRequest, Headers, ImageGenerationOperations, Params,
};
use super::super::plan::{ImageGenerationPlan, MappedOptions, PreparedCall};

const DEFAULT_AZURE_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

struct Credentials {
    api_base: Option<String>,
    api_version: Option<String>,
    api_key: Option<SecretString>,
    azure_ad_token: Option<SecretString>,
}

pub(super) async fn send<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    mapped: &MappedOptions,
    call: &ImageGenerationCall,
    logging: &mut LoggingState,
) -> Result<BranchAnswer, ImageGenerationException>
where
    Identity: Sync,
    Operations: ImageGenerationOperations + Sync,
{
    let mut optional_params = mapped.params.clone();
    let credentials = credentials(plan, prepared, call, &mut optional_params);
    let token_provider = entra_provider(plan, prepared);
    let mut headers = mapped.headers.clone();
    insert_default_headers(&mut headers, credentials.api_key.as_ref());
    plan.operations
        .send(
            Branch::Azure(AzureRequest {
                model: prepared.model.as_deref(),
                prompt: &call.prompt,
                timeout: prepared.timeout,
                api_key: credentials.api_key.as_ref(),
                api_base: credentials.api_base.as_deref(),
                api_version: credentials.api_version.as_deref(),
                azure_ad_token: credentials.azure_ad_token.as_ref(),
                azure_ad_token_provider: token_provider.as_ref(),
                optional_params: &optional_params,
                headers: &headers,
                litellm_params: &mapped.litellm_params,
                base_model: prepared.generic.base_model.as_deref(),
                asynchronous: prepared.generic.aimg_generation,
                now_seconds: call.now_seconds,
            }),
            logging,
        )
        .await
        .map_err(|failure| raise(plan.redactor, *failure))
}

fn credentials<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
    call: &ImageGenerationCall,
    optional_params: &mut Params,
) -> Credentials
where
    Operations: ImageGenerationOperations,
{
    // the call's option or kwarg wins only as a nonempty string; every other value falls back to AZURE_AD_TOKEN
    let popped = optional_params.shift_remove("azure_ad_token");
    let folded = take_extra_body_token(optional_params);
    let from_options = popped.or(folded);
    let from_kwargs = prepared.generic.azure_ad_token.clone();
    let azure_ad_token = match from_options.or(from_kwargs) {
        Some(Json::Text(text)) if !text.is_empty() => Some(SecretString::from(text)),
        Some(
            Json::Text(_)
            | Json::Null
            | Json::Bool(_)
            | Json::Number(_)
            | Json::List(_)
            | Json::Object(_),
        )
        | None => plan.settings.azure_ad_token.clone(),
    };
    Credentials {
        api_base: prepared
            .api_base
            .clone()
            .or_else(|| plan.settings.api_base.clone())
            .or_else(|| plan.settings.azure_api_base.clone()),
        api_version: call
            .api_version
            .clone()
            .or_else(|| plan.settings.api_version.clone())
            .or_else(|| plan.settings.azure_api_version.clone()),
        api_key: call
            .api_key
            .clone()
            .or_else(|| plan.settings.api_key.clone())
            .or_else(|| plan.settings.azure_key.clone())
            .or_else(|| plan.settings.azure_openai_api_key.clone())
            .or_else(|| plan.settings.azure_api_key.clone()),
        azure_ad_token,
    }
}

// get_optional_params_image_gen folds azure_ad_token into extra_body, which Azure flattens into the body
fn take_extra_body_token(optional_params: &mut Params) -> Option<Json> {
    let Some(Json::Object(fields)) = optional_params.get_mut("extra_body") else {
        return None;
    };
    let removed = fields.shift_remove("azure_ad_token");
    let emptied = fields.is_empty();
    if emptied {
        optional_params.shift_remove("extra_body");
    }
    removed
}

// the default loop checks the exact spelling, so a differently cased caller key never blocks it
fn insert_default_headers(headers: &mut Headers, api_key: Option<&SecretString>) {
    let mut defaults = Headers::new();
    defaults.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    if let Some(key) = api_key {
        defaults.insert(String::from("api-key"), key.expose_secret().to_owned());
    }
    for (name, value) in defaults {
        headers.entry(name).or_insert(value);
    }
}

fn entra_provider<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    prepared: &PreparedCall,
) -> Option<Arc<dyn CallableTokenProvider>>
where
    Operations: ImageGenerationOperations,
{
    if let Some(provider) = &plan.azure_ad_token_provider {
        return Some(Arc::clone(provider));
    }
    // a callback the caller passed through kwargs is already in the plan, or is not ours to build
    if prepared.generic.azure_ad_token_provider {
        return None;
    }
    let credentials = &prepared.generic.credentials;
    let (Some(tenant_id), Some(client_id), Some(client_secret)) = (
        credentials.tenant_id.as_deref(),
        credentials.client_id.as_deref(),
        credentials.client_secret.as_ref(),
    ) else {
        return None;
    };
    if tenant_id.is_empty() || client_id.is_empty() || client_secret.expose_secret().is_empty() {
        return None;
    }
    let scope = credentials
        .azure_scope
        .as_deref()
        .filter(|scope| !scope.is_empty())
        .unwrap_or(DEFAULT_AZURE_SCOPE);
    plan.operations.entra_token_provider(EntraRequest {
        tenant_id,
        client_id,
        client_secret: client_secret.expose_secret(),
        scope,
    })
}
