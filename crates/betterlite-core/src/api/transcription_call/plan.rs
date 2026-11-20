use betterlite_config::sections::transcription_call::Settings;
use betterlite_context::media_config_selection::audio_transcription::{
    AudioTranscriptionConfigTag, select_transcription_config,
};
use betterlite_context::media_optional_params::transcription::{
    TranscriptionRequest, transcription_optional_params,
};
use betterlite_context::non_default_params::Params;
use betterlite_context::non_default_params::filters::{Endpoint, non_default_kwargs};
use betterlite_context::non_default_params::mapper::DropPolicy;
use betterlite_models::catalog::ModelCatalog;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::resolution::{
    ProviderIdentity, ResolutionContext, ResolutionRequest, resolve,
};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Number;

use crate::api::resource_sdk_settings::{ResourceSdkRequestOptions, ResourceSdkSettings};

use super::call::TranscriptionCall;
use super::dispatch::TranscriptionDispatch;
use super::error::TranscriptionError;
use super::is_native_endpoint;
use super::is_openai_audio_provider;

// everything the provider branches share: the resolved identity, the selected config and the mapped options
pub(super) struct Plan {
    pub model: String,
    pub slug: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub config: Option<AudioTranscriptionConfigTag>,
    pub optional_params: Params,
    // the kwargs after atranscription, tags and client were popped, as get_litellm_params sees them
    pub kwargs: Params,
    pub openai_transport: bool,
}

pub(super) fn prepare<Dispatch>(
    call: &TranscriptionCall,
    dispatch: &Dispatch,
    settings: &Settings,
    resolution: &ResolutionContext<'_>,
    bundled: &ModelCatalog,
) -> Result<Plan, TranscriptionError<Dispatch::Error>>
where
    Dispatch: TranscriptionDispatch,
{
    let requested_base = call.api_base.clone().or_else(|| call.base_url.clone());
    let request = ResolutionRequest {
        model: &call.model,
        custom_llm_provider: call.custom_llm_provider.as_deref(),
        api_base: requested_base.as_deref(),
        api_key: call.api_key.as_ref(),
    };
    let resolved = resolve(&request, resolution)?;
    let slug = resolved.provider.slug().to_owned();
    let config = match &resolved.provider {
        ProviderIdentity::Known(provider) => {
            select_transcription_config(&resolved.model, *provider, resolution.catalog, bundled)
        }
        ProviderIdentity::Anyscale | ProviderIdentity::Custom(_) => None,
    };
    let openai_transport = is_openai_audio_provider(&slug) && !is_native_endpoint(config);
    let kwargs = popped_kwargs(call);
    let passed = passed_params(&kwargs, call);
    let drop_policy = DropPolicy {
        drop_params: call.params.get("drop_params").and_then(bool_value),
        litellm_drop_params: settings.drop_params.unwrap_or(false),
    };
    let optional_params = transcription_optional_params(&TranscriptionRequest {
        model: &resolved.model,
        custom_llm_provider: &slug,
        params: &passed,
        drop_policy,
        mapper: dispatch.mapper(&slug, config),
    })?;
    Ok(Plan {
        model: resolved.model,
        slug,
        api_key: resolved.api_key.or_else(|| call.api_key.clone()),
        api_base: resolved.api_base,
        config,
        optional_params,
        kwargs,
        openai_transport,
    })
}

// the three kwargs the signature pops before get_litellm_params and the optional params read the rest
fn popped_kwargs(call: &TranscriptionCall) -> Params {
    let mut kwargs = call.params.clone();
    kwargs.remove("tags");
    kwargs.remove("atranscription");
    kwargs.remove("client");
    kwargs
}

// get_non_default_transcription_params plus the five signature options get_optional_params_transcription reads
fn passed_params(kwargs: &Params, call: &TranscriptionCall) -> Params {
    let mut passed = non_default_kwargs(kwargs, Endpoint::Transcription);
    passed.insert(
        String::from("language"),
        optional_text(call.language.as_deref()),
    );
    passed.insert(
        String::from("prompt"),
        optional_text(call.prompt.as_deref()),
    );
    passed.insert(
        String::from("response_format"),
        optional_text(call.response_format.as_deref()),
    );
    passed.insert(
        String::from("timestamp_granularities"),
        call.timestamp_granularities
            .as_ref()
            .map_or(JsonValue::Null, |values| {
                JsonValue::Array(values.iter().cloned().map(JsonValue::Text).collect())
            }),
    );
    passed.insert(
        String::from("temperature"),
        call.temperature.map_or(JsonValue::Null, |value| {
            JsonValue::Number(Number::from(value))
        }),
    );
    passed
}

fn optional_text(value: Option<&str>) -> JsonValue {
    value.map_or(JsonValue::Null, |text| JsonValue::Text(text.to_owned()))
}

fn bool_value(value: &JsonValue) -> Option<bool> {
    match value {
        JsonValue::Bool(flag) => Some(*flag),
        JsonValue::Null
        | JsonValue::Number(_)
        | JsonValue::Text(_)
        | JsonValue::Array(_)
        | JsonValue::Object(_) => None,
    }
}

// the openai branch: api_base, key and organization, each through its own fallback chain
pub(super) struct OpenAiCredentials {
    pub api_base: String,
    pub api_key: Option<SecretString>,
    pub organization: Option<String>,
}

pub(super) fn openai_credentials(plan: &Plan, settings: &Settings) -> OpenAiCredentials {
    let resolved = ResourceSdkSettings::from(settings);
    let options = ResourceSdkRequestOptions {
        api_base: plan.api_base.clone(),
        api_key: plan.api_key.clone(),
        ..ResourceSdkRequestOptions::default()
    };
    OpenAiCredentials {
        api_base: resolved.openai_base(&options),
        api_key: resolved.openai_api_key(&options).cloned(),
        organization: first_text([
            resolved.organization.as_deref(),
            resolved.openai_organization.as_deref(),
        ]),
    }
}

// the azure branch: its key chain never reads AZURE_OPENAI_API_KEY, unlike speech's
pub(super) struct AzureCredentials {
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub api_key: Option<SecretString>,
    pub azure_ad_token: Option<SecretString>,
}

pub(super) fn azure_credentials(
    plan: &Plan,
    call: &TranscriptionCall,
    settings: &Settings,
) -> AzureCredentials {
    let resolved = ResourceSdkSettings::from(settings);
    let options = ResourceSdkRequestOptions {
        api_base: plan.api_base.clone(),
        api_version: call.api_version.clone(),
        api_key: plan.api_key.clone(),
        ..ResourceSdkRequestOptions::default()
    };
    AzureCredentials {
        api_base: resolved.azure_base(&options).map(str::to_owned),
        api_version: resolved.azure_version(&options).map(str::to_owned),
        api_key: first_secret([
            options.api_key.as_ref(),
            resolved.api_key.as_ref(),
            resolved.azure_key.as_ref(),
            resolved.azure_api_key.as_ref(),
        ]),
        azure_ad_token: first_secret([
            call.azure_ad_token.as_ref(),
            resolved.azure_ad_token.as_ref(),
        ]),
    }
}

fn first_text<const N: usize>(candidates: [Option<&str>; N]) -> Option<String> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn first_secret<const N: usize>(candidates: [Option<&SecretString>; N]) -> Option<SecretString> {
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.expose_secret().is_empty())
        .cloned()
}

impl From<&Settings> for ResourceSdkSettings {
    fn from(settings: &Settings) -> Self {
        Self {
            api_base: settings.api_base.clone(),
            api_version: settings.api_version.clone(),
            api_key: settings.api_key.clone(),
            openai_key: settings.openai_key.clone(),
            azure_key: settings.azure_key.clone(),
            azure_ad_token: settings.azure_ad_token.clone(),
            organization: settings.organization.clone(),
            vertex_project: None,
            vertex_location: None,
            openai_base_url: settings.openai_base_url.clone(),
            openai_api_base: settings.openai_api_base.clone(),
            openai_api_key: settings.openai_api_key.clone(),
            openai_organization: settings.openai_organization.clone(),
            azure_api_base: settings.azure_api_base.clone(),
            azure_api_version: settings.azure_api_version.clone(),
            azure_openai_api_key: settings.azure_openai_api_key.clone(),
            azure_api_key: settings.azure_api_key.clone(),
            vertexai_project: None,
            vertexai_location: None,
            vertexai_credentials: None,
        }
    }
}
