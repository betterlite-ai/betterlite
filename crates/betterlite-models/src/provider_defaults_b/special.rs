use crate::identity::provider_id::ProviderId;
use crate::provider_defaults_b::environment::EnvironmentLookup;
use crate::provider_defaults_b::error::ProviderDefaultsError;
use crate::provider_defaults_b::globals::GlobalCredentials;

fn chain<E>(explicit: Option<&str>, global: Option<&str>, env_var: &str, env: &E) -> Option<String>
where
    E: EnvironmentLookup,
{
    explicit
        .map(ToOwned::to_owned)
        .or_else(|| global.map(ToOwned::to_owned))
        .or_else(|| env.lookup(env_var))
}

// Shares the base/key fallback chain between xai, inception and lemonade: only names and guards differ.
pub struct SpecialDefaults {
    pub base_env_var: &'static str,
    pub default_base: &'static str,
    pub key_env_var: &'static str,
    pub global_key: fn(&GlobalCredentials) -> Option<&str>,
    // An explicit base without an explicit key skips every credential fallback, matching litellm's guard.
    pub guard_key_on_base: bool,
    pub default_key: Option<&'static str>,
}

pub const SPECIAL_TABLE: &[(ProviderId, SpecialDefaults)] = &[
    (
        ProviderId::Xai,
        SpecialDefaults {
            base_env_var: "XAI_API_BASE",
            default_base: "https://api.x.ai/v1",
            key_env_var: "XAI_API_KEY",
            global_key: |globals| globals.xai_key.as_deref(),
            guard_key_on_base: false,
            default_key: None,
        },
    ),
    (
        ProviderId::Inception,
        SpecialDefaults {
            base_env_var: "INCEPTION_API_BASE",
            default_base: "https://api.inceptionlabs.ai/v1",
            key_env_var: "INCEPTION_API_KEY",
            global_key: |globals| globals.inception_key.as_deref(),
            guard_key_on_base: true,
            default_key: None,
        },
    ),
    (
        ProviderId::Lemonade,
        SpecialDefaults {
            base_env_var: "LEMONADE_API_BASE",
            default_base: "http://localhost:8000/api/v1",
            key_env_var: "LEMONADE_API_KEY",
            global_key: |globals| globals.lemonade_key.as_deref(),
            guard_key_on_base: true,
            default_key: Some("lemonade"),
        },
    ),
];

pub fn resolve_special<E>(
    api_base: Option<&str>,
    api_key: Option<&str>,
    defaults: &SpecialDefaults,
    globals: &GlobalCredentials,
    env: &E,
) -> (Option<String>, Option<String>)
where
    E: EnvironmentLookup,
{
    let base = Some(
        chain(api_base, None, defaults.base_env_var, env)
            .unwrap_or_else(|| defaults.default_base.to_owned()),
    );
    let key_allowed = !defaults.guard_key_on_base || api_base.is_none() || api_key.is_some();
    let key = key_allowed
        .then(|| {
            chain(
                api_key,
                (defaults.global_key)(globals),
                defaults.key_env_var,
                env,
            )
        })
        .flatten()
        .or_else(|| defaults.default_key.map(ToOwned::to_owned));
    (base, key)
}

// Mirrors RAGFlowConfig._parse_ragflow_model: a valid model is ragflow/{chat|agent}/{id}/{model...}.
fn validate_ragflow_model(model: &str) -> Result<(), ProviderDefaultsError> {
    let parts: Vec<&str> = model.split('/').collect();
    let valid = parts.len() >= 4
        && parts.first() == Some(&"ragflow")
        && matches!(parts.get(1), Some(&"chat" | &"agent"));
    if valid {
        Ok(())
    } else {
        Err(ProviderDefaultsError::InvalidRagflowModel {
            model: model.to_owned(),
        })
    }
}

pub fn resolve_ragflow<E>(
    model: &str,
    api_base: Option<&str>,
    api_key: Option<&str>,
    globals: &GlobalCredentials,
    env: &E,
) -> Result<(String, Option<String>, Option<String>), ProviderDefaultsError>
where
    E: EnvironmentLookup,
{
    let rewritten = format!("ragflow/{model}");
    validate_ragflow_model(&rewritten)?;
    let base = chain(
        api_base,
        globals.api_base.as_deref(),
        "RAGFLOW_API_BASE",
        env,
    );
    let global_key = globals
        .api_key
        .as_ref()
        .map(|key| secrecy::ExposeSecret::expose_secret(key).to_owned());
    let key = chain(api_key, global_key.as_deref(), "RAGFLOW_API_KEY", env);
    Ok((rewritten, base, key))
}
