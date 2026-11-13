use betterlite_models::compatible_api_selection::{
    CompatibleApiPolicy, MessagesApiPolicy, ProviderIdentity, ResponsesApiPolicy,
    embedding_api_policy, image_generation_api_policy, messages_api_policy, rerank_api_policy,
    responses_api_policy,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{JSONProviderRegistry, ProviderDescriptor};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

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
    type Input = Vec<Call>;

    fn run(input: &Self::Input) -> HelperResult {
        let reports = input
            .iter()
            .map(report)
            .collect::<Result<Vec<_>, Box<HelperError>>>()?;
        Ok(Value::Array(reports))
    }
}
fn report(call: &Call) -> Result<Value, Box<HelperError>> {
    let value = match call.family {
        Family::Responses => responses_report(call)?,
        Family::Messages => plain(
            messages_api_policy(provider(call)?, &registry(call)?)
                .map(MessagesApiPolicy::class_name),
        ),
        Family::Embedding => {
            plain(embedding_api_policy(provider(call)?).map(CompatibleApiPolicy::class_name))
        }
        Family::Rerank => {
            plain(rerank_api_policy(provider(call)?).map(CompatibleApiPolicy::class_name))
        }
        Family::ImageGeneration => {
            plain(image_generation_api_policy(provider(call)?).map(CompatibleApiPolicy::class_name))
        }
    };
    Ok(value)
}
fn responses_report(call: &Call) -> Result<Value, Box<HelperError>> {
    let identity = match call.identity {
        IdentityKind::Name => ProviderIdentity::Name(&call.provider),
        IdentityKind::Enum => ProviderIdentity::Enum(provider(call)?),
    };
    let native = call.native;
    let selector = |known: Option<ProviderId>| {
        (native == known && known.is_some()).then_some(ResponsesApiPolicy::Native)
    };
    let value = match responses_api_policy(identity, selector, &registry(call)?) {
        None => Value::Null,
        Some(ResponsesApiPolicy::Native) => Value::String(String::from("native")),
        Some(ResponsesApiPolicy::Json(provider)) => Value::String(format!("json:{provider}")),
    };
    Ok(value)
}
fn provider(call: &Call) -> Result<ProviderId, Box<HelperError>> {
    call.provider.parse().map_err(|error| {
        invalid(format!(
            "{} is not a provider identity: {error}",
            call.provider
        ))
    })
}
fn plain(value: Option<&'static str>) -> Value {
    match value {
        Some(name) => Value::String(name.to_owned()),
        None => Value::Null,
    }
}
fn registry(call: &Call) -> Result<JSONProviderRegistry, Box<HelperError>> {
    let mut providers: IndexMap<ProviderId, ProviderDescriptor> = IndexMap::new();
    for entry in &call.providers {
        let fixture = serde_json::json!({
            "base_url": "",
            "api_key_env": "",
            "supported_endpoints": &entry.supported_endpoints,
        });
        let config = serde_json::from_value(fixture)
            .map_err(|error| invalid(format!("the descriptor fixture is invalid: {error}")))?;
        providers.insert(
            entry.provider,
            ProviderDescriptor {
                provider: entry.provider,
                config,
            },
        );
    }
    Ok(JSONProviderRegistry {
        providers,
        loaded: true,
    })
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
