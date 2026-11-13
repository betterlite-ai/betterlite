use std::collections::BTreeMap;

use betterlite_models::compatible_api_selection::{
    EmbeddingApiPolicy, ImageGenerationApiPolicy, MessagesApiPolicy, ProviderIdentity,
    RerankApiPolicy, ResponsesApiPolicy, embedding_api_policy, image_generation_api_policy,
    messages_api_policy, rerank_api_policy, responses_api_policy,
};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    BaseClass, Constraints, JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};
use indexmap::IndexMap;
use serde::Deserialize;
use serde_json::Value;

use crate::helpers::{Helper, HelperError, HelperResult};

mod tests;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    calls: Vec<Call>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
enum Call {
    Responses {
        provider: String,
        #[serde(default)]
        identity: IdentityKind,
        #[serde(default)]
        native: Option<ProviderId>,
        #[serde(default)]
        providers: Vec<Descriptor>,
    },
    Messages {
        provider: ProviderId,
        #[serde(rename = "model")]
        _model: String,
        #[serde(default)]
        providers: Vec<Descriptor>,
    },
    Embedding {
        provider: ProviderId,
        #[serde(rename = "model")]
        _model: String,
    },
    Rerank {
        provider: ProviderId,
        #[serde(rename = "model")]
        _model: String,
    },
    ImageGeneration {
        provider: ProviderId,
        #[serde(rename = "model")]
        _model: String,
    },
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
            .map(report)
            .collect::<Result<Vec<_>, Box<HelperError>>>()?;
        Ok(Value::Array(reports))
    }
}

fn report(call: &Call) -> Result<Value, Box<HelperError>> {
    match call {
        Call::Responses {
            provider,
            identity,
            native,
            providers,
        } => responses_report(provider, identity, *native, providers),
        Call::Messages {
            provider,
            providers,
            ..
        } => Ok(option_name(
            messages_api_policy(*provider, &registry(providers)).map(MessagesApiPolicy::class_name),
        )),
        Call::Embedding { provider, .. } => Ok(option_name(
            embedding_api_policy(*provider).map(EmbeddingApiPolicy::class_name),
        )),
        Call::Rerank { provider, .. } => Ok(option_name(
            rerank_api_policy(*provider).map(RerankApiPolicy::class_name),
        )),
        Call::ImageGeneration { provider, .. } => Ok(option_name(
            image_generation_api_policy(*provider).map(ImageGenerationApiPolicy::class_name),
        )),
    }
}

fn responses_report(
    provider: &str,
    identity: &IdentityKind,
    native: Option<ProviderId>,
    providers: &[Descriptor],
) -> Result<Value, Box<HelperError>> {
    let identity = match identity {
        IdentityKind::Name => ProviderIdentity::Name(provider),
        IdentityKind::Enum => match provider.parse() {
            Ok(parsed) => ProviderIdentity::Enum(parsed),
            Err(error) => {
                return Err(invalid(format!(
                    "{provider} is not a provider identity: {error}"
                )));
            }
        },
    };
    let selector = |known: Option<ProviderId>| {
        (native == known && known.is_some()).then_some(ResponsesApiPolicy::Native)
    };
    Ok(
        match responses_api_policy(identity, selector, &registry(providers)) {
            None => Value::Null,
            Some(ResponsesApiPolicy::Native) => Value::String(String::from("native")),
            Some(ResponsesApiPolicy::Json(provider)) => Value::String(format!("json:{provider}")),
        },
    )
}

fn option_name(value: Option<&'static str>) -> Value {
    match value {
        Some(name) => Value::String(name.to_owned()),
        None => Value::Null,
    }
}

fn registry(descriptors: &[Descriptor]) -> JSONProviderRegistry {
    let providers: IndexMap<ProviderId, ProviderDescriptor> = descriptors
        .iter()
        .map(|descriptor| {
            let provider = descriptor.provider;
            let entry = ProviderDescriptor {
                provider,
                config: ProviderConfig {
                    base_url: String::new(),
                    api_key_env: String::new(),
                    api_base_env: None,
                    base_class: BaseClass::default(),
                    param_mappings: BTreeMap::new(),
                    constraints: Constraints::default(),
                    special_handling: BTreeMap::new(),
                    supported_endpoints: descriptor.supported_endpoints.clone(),
                },
            };
            (provider, entry)
        })
        .collect();
    JSONProviderRegistry {
        providers,
        loaded: true,
    }
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
