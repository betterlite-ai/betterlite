use betterlite_models::compatible_api_selection::{
    ProviderIdentity, ResponsesApiPolicy, embedding_api_policy, image_generation_api_policy,
    messages_api_policy, rerank_api_policy, responses_api_policy,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
struct Call {
    family: Family,
    provider: String,
    #[serde(default)]
    identity: IdentityKind,
    #[serde(default)]
    native: Option<ProviderId>,
    #[serde(default)]
    providers: Vec<Descriptor>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Family {
    Responses,
    Messages,
    Embedding,
    Rerank,
    ImageGeneration,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum IdentityKind {
    #[default]
    Name,
    Enum,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    provider: ProviderId,
    #[serde(default)]
    supported_endpoints: Vec<String>,
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let reports = input
            .calls
            .iter()
            .map(|call| answer(call).map(|report| report.map_or(Value::Null, Value::String)))
            .collect::<Result<Vec<_>, Box<HelperError>>>()?;
        Ok(Value::Array(reports))
    }
}

fn answer(call: &Call) -> Result<Option<String>, Box<HelperError>> {
    match call.family {
        Family::Responses => responses_answer(call),
        Family::Messages => Ok(
            messages_api_policy(provider(&call.provider)?, &registry(call)?)
                .map(|policy| policy.class_name().to_owned()),
        ),
        Family::Embedding => Ok(embedding_api_policy(provider(&call.provider)?)
            .map(|policy| policy.class_name().to_owned())),
        Family::Rerank => Ok(rerank_api_policy(provider(&call.provider)?)
            .map(|policy| policy.class_name().to_owned())),
        Family::ImageGeneration => Ok(image_generation_api_policy(provider(&call.provider)?)
            .map(|policy| policy.class_name().to_owned())),
    }
}

fn responses_answer(call: &Call) -> Result<Option<String>, Box<HelperError>> {
    let selected = responses_api_policy(
        identity(call)?,
        |known| native_answer(call, known),
        &registry(call)?,
    );
    Ok(selected.map(|policy| match policy {
        ResponsesApiPolicy::Native => String::from("native"),
        ResponsesApiPolicy::Json(slug) => format!("json:{slug}"),
    }))
}

fn identity(call: &Call) -> Result<ProviderIdentity<'_>, Box<HelperError>> {
    match call.identity {
        IdentityKind::Name => Ok(ProviderIdentity::Name(call.provider.as_str())),
        IdentityKind::Enum => Ok(ProviderIdentity::Enum(provider(&call.provider)?)),
    }
}

fn native_answer(call: &Call, known: Option<ProviderId>) -> Option<ResponsesApiPolicy> {
    let declared = call.native?;
    if Some(declared) != known {
        return None;
    }
    Some(ResponsesApiPolicy::Native)
}

fn provider(slug: &str) -> Result<ProviderId, Box<HelperError>> {
    slug.parse()
        .map_err(|error| invalid(format!("{slug} is not a provider identity: {error}")))
}

fn registry(call: &Call) -> Result<JSONProviderRegistry, Box<HelperError>> {
    let mut registry = JSONProviderRegistry::default();
    let fixtures = call
        .providers
        .iter()
        .map(fixture)
        .collect::<Result<IndexMap<ProviderId, ProviderDescriptor>, Box<HelperError>>>()?;
    registry.providers.extend(fixtures);
    registry.loaded = true;
    Ok(registry)
}

fn fixture(entry: &Descriptor) -> Result<(ProviderId, ProviderDescriptor), Box<HelperError>> {
    let config = serde_json::from_value(serde_json::json!({
        "base_url": "",
        "api_key_env": "",
        "supported_endpoints": &entry.supported_endpoints,
    }))
    .map_err(|error| invalid(format!("the descriptor fixture is invalid: {error}")))?;
    Ok((
        entry.provider,
        ProviderDescriptor {
            provider: entry.provider,
            config,
        },
    ))
}

fn invalid(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: String::from("InvalidInput"),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}
