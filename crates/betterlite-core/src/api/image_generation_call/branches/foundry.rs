use betterlite_logging::logging_state::LoggingState;
use secrecy::SecretString;

use super::super::call::ImageGenerationCall;
use super::super::error::{ImageGenerationException, raise};
use super::super::operations::{
    Branch, BranchAnswer, FoundryAuthRequest, FoundryRequest, Headers, ImageGenerationOperations,
    Params,
};
use super::super::plan::{ImageGenerationPlan, MappedOptions, PreparedCall};
use super::header_object;

struct Credentials {
    api_base: Option<String>,
    api_key: Option<SecretString>,
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
    let credentials = credentials(plan, prepared, call);
    let optional_params = optional_params(prepared, mapped);
    let headers = request_headers(plan, mapped, credentials.api_key.as_ref())?;
    plan.operations
        .send(
            Branch::Foundry(FoundryRequest {
                model: prepared.model.as_deref(),
                prompt: &call.prompt,
                timeout: prepared.timeout,
                api_key: credentials.api_key.as_ref(),
                api_base: credentials.api_base.as_deref(),
                api_version: call.api_version.as_deref(),
                azure_ad_token: None,
                azure_ad_token_provider: plan.azure_ad_token_provider.as_ref(),
                optional_params: &optional_params,
                headers: &headers,
                litellm_params: &mapped.litellm_params,
                asynchronous: prepared.generic.aimg_generation,
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
) -> Credentials
where
    Operations: ImageGenerationOperations,
{
    Credentials {
        api_base: prepared
            .api_base
            .clone()
            .or_else(|| plan.settings.api_base.clone())
            .or_else(|| plan.settings.azure_ai_api_base.clone()),
        api_key: call
            .api_key
            .clone()
            .or_else(|| plan.settings.api_key.clone())
            .or_else(|| plan.settings.azure_ai_api_key.clone()),
    }
}

// extra_headers stays in optional_params for the handler and is also merged into the request headers
fn optional_params(prepared: &PreparedCall, mapped: &MappedOptions) -> Params {
    let mut params = mapped.params.clone();
    if let Some(extra) = &prepared.generic.extra_headers {
        params.insert(String::from("extra_headers"), header_object(extra));
    }
    params
}

// a caller's api-key or authorization header is detected case-insensitively and always wins
fn request_headers<Identity, Operations>(
    plan: &ImageGenerationPlan<'_, Identity, Operations>,
    mapped: &MappedOptions,
    api_key: Option<&SecretString>,
) -> Result<Headers, ImageGenerationException>
where
    Operations: ImageGenerationOperations,
{
    let caller_set_auth = mapped.headers.keys().any(|name| {
        name.eq_ignore_ascii_case("api-key") || name.eq_ignore_ascii_case("authorization")
    });
    let auth_headers = if caller_set_auth {
        mapped.headers.clone()
    } else {
        plan.operations
            .foundry_auth_headers(FoundryAuthRequest {
                api_key,
                litellm_params: &mapped.litellm_params,
            })
            .map_err(|failure| raise(plan.redactor, *failure))?
    };
    let mut headers = Headers::new();
    headers.insert(
        String::from("Content-Type"),
        String::from("application/json"),
    );
    headers.extend(auth_headers);
    headers.extend(mapped.headers.clone());
    Ok(headers)
}
