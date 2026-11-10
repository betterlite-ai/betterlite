use secrecy::SecretString;

use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_a::environment::{EnvironmentLookup, first_set};
use crate::provider_defaults_a::error::ProviderDefaultsError;
use crate::provider_defaults_a::model::{
    DefaultsProvider, ProviderDefaultsRequest, ResolvedProvider,
};
use crate::provider_defaults_a::policies::{MantleEndpoint, ModelSets};
use crate::provider_defaults_a::table::{SimpleDefaults, resolve_key, simple_defaults};
use crate::provider_defaults_a::url;

pub struct KeyPolicy {
    pub env: &'static [&'static str],
    pub default: Option<&'static str>,
}

pub struct BasetenDefaults {
    pub default_base: &'static str,
    pub dedicated_prefix: &'static str,
    pub dedicated_suffix: &'static str,
    pub dedicated_name_length: usize,
}

pub struct DatarobotDefaults {
    pub base_env: &'static [&'static str],
    pub default_base: &'static str,
    pub llmgw_path: &'static str,
    pub llmgw_suffix: &'static str,
}

pub enum BasePolicy {
    // A base equal to the provider's own endpoint keeps the environment key; any other caller base hides it.
    Defaulted {
        env: &'static [&'static str],
        default: &'static str,
    },
    ModelDerived(BasetenDefaults),
    Datarobot(DatarobotDefaults),
}

pub struct GuardedDefaults {
    pub base: BasePolicy,
    pub key: KeyPolicy,
}

pub enum SpecialDefaults {
    PassThrough,
    Guarded(GuardedDefaults),
    Ai21(SimpleDefaults),
    AzureAi,
    Mantle,
}

const BASETEN: GuardedDefaults = GuardedDefaults {
    base: BasePolicy::ModelDerived(BasetenDefaults {
        default_base: "https://inference.baseten.co/v1",
        dedicated_prefix: "https://model-",
        dedicated_suffix: ".api.baseten.co/environments/production/sync/v1",
        dedicated_name_length: 8,
    }),
    key: KeyPolicy {
        env: &["BASETEN_API_KEY"],
        default: None,
    },
};

const DATAROBOT: GuardedDefaults = GuardedDefaults {
    base: BasePolicy::Datarobot(DatarobotDefaults {
        base_env: &["DATAROBOT_ENDPOINT"],
        default_base: "https://app.datarobot.com",
        llmgw_path: "/genai/llmgw/chat/completions",
        llmgw_suffix: "/api/v2/genai/llmgw/chat/completions",
    }),
    key: KeyPolicy {
        env: &["DATAROBOT_API_TOKEN"],
        default: Some("fake-api-key"),
    },
};

const NADIR: GuardedDefaults = GuardedDefaults {
    base: BasePolicy::Defaulted {
        env: &["NADIR_API_BASE"],
        default: "https://api.getnadir.com/v1",
    },
    key: KeyPolicy {
        env: &["NADIR_API_KEY"],
        default: None,
    },
};

const AI21: SimpleDefaults = SimpleDefaults {
    base_env: &["AI21_API_BASE"],
    base_default: Some("https://api.ai21.com/studio/v1"),
    key_env: &["AI21_API_KEY"],
    key_default: None,
};

pub(crate) const SPECIAL_TABLE: &[(ProviderId, SpecialDefaults)] = &[
    (ProviderId::AiohttpOpenai, SpecialDefaults::PassThrough),
    (ProviderId::Nadir, SpecialDefaults::Guarded(NADIR)),
    (ProviderId::Baseten, SpecialDefaults::Guarded(BASETEN)),
    (ProviderId::Datarobot, SpecialDefaults::Guarded(DATAROBOT)),
    (ProviderId::Ai21, SpecialDefaults::Ai21(AI21)),
    (ProviderId::Ai21Chat, SpecialDefaults::Ai21(AI21)),
    (ProviderId::AzureAi, SpecialDefaults::AzureAi),
    (ProviderId::BedrockMantle, SpecialDefaults::Mantle),
];

pub(crate) fn guarded<E>(
    provider: ProviderId,
    request: ProviderDefaultsRequest,
    defaults: &GuardedDefaults,
    env: &E,
) -> Result<ResolvedProvider, ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    let (api_base, trusted) = guarded_base(&request, defaults, env)?;
    let key_env: &[&str] = if trusted { defaults.key.env } else { &[] };
    let explicit = request
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let api_key = resolve_key(explicit, key_env, defaults.key.default, env);
    Ok(ResolvedProvider {
        provider: DefaultsProvider::Known(provider),
        model: request.model,
        api_base,
        api_key: api_key.map(SecretString::from),
    })
}

fn guarded_base<E>(
    request: &ProviderDefaultsRequest,
    defaults: &GuardedDefaults,
    env: &E,
) -> Result<(Option<String>, bool), ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    match &defaults.base {
        BasePolicy::Defaulted {
            env: names,
            default,
        } => {
            let own_base = first_set(env, names).unwrap_or_else(|| (*default).to_owned());
            let trusted = request
                .api_base
                .as_deref()
                .is_none_or(|value| value.trim_end_matches('/') == own_base.trim_end_matches('/'));
            Ok((Some(request.api_base.clone().unwrap_or(own_base)), trusted))
        }
        BasePolicy::ModelDerived(policy) => Ok((
            Some(
                request
                    .api_base
                    .clone()
                    .unwrap_or_else(|| baseten_base(&request.model, policy)),
            ),
            true,
        )),
        BasePolicy::Datarobot(policy) => {
            let env_base =
                first_set(env, policy.base_env).unwrap_or_else(|| policy.default_base.to_owned());
            let source = request.api_base.as_deref().unwrap_or(env_base.as_str());
            let base = datarobot_base(source, policy)?;
            Ok((Some(base), true))
        }
    }
}

// BasetenConfig.get_api_base_for_model sends an 8-character deployment id to its dedicated host.
fn baseten_base(model: &str, defaults: &BasetenDefaults) -> String {
    let deployment = model.replace("baseten/", "");
    let dedicated = deployment.chars().count() == defaults.dedicated_name_length
        && deployment
            .chars()
            .all(|character| character.is_ascii_alphanumeric());
    if dedicated {
        format!(
            "{}{deployment}{}",
            defaults.dedicated_prefix, defaults.dedicated_suffix
        )
    } else {
        defaults.default_base.to_owned()
    }
}

// DataRobotConfig._resolve_api_base appends the LLM gateway path to the endpoint the caller configured.
fn datarobot_base(
    source: &str,
    defaults: &DatarobotDefaults,
) -> Result<String, ProviderDefaultsError> {
    let mut parts =
        url::parse(source).map_err(|url_error| ProviderDefaultsError::InvalidApiBase {
            api_base: source.to_owned(),
            source: url_error,
        })?;
    let path = &parts.path;
    let mut resolved = if path.is_empty() || path == "/" {
        format!("{path}{}", defaults.llmgw_suffix)
    } else if path.contains("api/v2/deployments") {
        path.clone()
    } else if path.contains("api/v2") && !path.contains(defaults.llmgw_path) {
        format!("{path}{}", defaults.llmgw_path)
    } else {
        path.clone()
    };
    if !resolved.ends_with('/') {
        resolved.push('/');
    }
    parts.path = resolved.replace("//", "/");
    Ok(url::unparse(&parts))
}

pub(crate) fn ai21<E>(
    provider: ProviderId,
    request: ProviderDefaultsRequest,
    defaults: &SimpleDefaults,
    models: &ModelSets,
    env: &E,
) -> ResolvedProvider
where
    E: EnvironmentLookup,
{
    let applies = provider == ProviderId::Ai21Chat
        || (provider == ProviderId::Ai21 && models.ai21_chat.contains(&request.model));
    if !applies {
        return ResolvedProvider {
            provider: DefaultsProvider::Known(provider),
            model: request.model,
            api_base: request.api_base,
            api_key: request.api_key,
        };
    }
    let api_key = request
        .api_key
        .as_ref()
        .map(secrecy::ExposeSecret::expose_secret);
    let (base, key) = simple_defaults(defaults, request.api_base.as_deref(), api_key, env);
    ResolvedProvider {
        provider: DefaultsProvider::Known(ProviderId::Ai21Chat),
        model: request.model,
        api_base: base,
        api_key: key.map(SecretString::from),
    }
}

pub(crate) fn mantle(
    provider: ProviderId,
    endpoint: Option<&MantleEndpoint>,
) -> Result<ResolvedProvider, ProviderDefaultsError> {
    let resolved = endpoint.ok_or(ProviderDefaultsError::MissingMantleEndpoint)?;
    Ok(ResolvedProvider {
        provider: DefaultsProvider::Known(provider),
        model: resolved.base_model.clone(),
        api_base: Some(resolved.api_base.clone()),
        api_key: resolved.api_key.clone(),
    })
}
