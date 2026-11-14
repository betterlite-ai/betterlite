use std::collections::BTreeMap;

use betterlite_models::sandbox_dispatch::SandboxToolEntry;
use betterlite_secret_managers::secret_lookup::SecretLookup;
use betterlite_secret_managers::secret_lookup::error::SecretLookupError;
use secrecy::{ExposeSecret as _, SecretString};
use thiserror::Error;

mod tests;

const OS_ENVIRON_PREFIX: &str = "os.environ/";

#[derive(Debug, Error)]
pub enum SandboxRegistryError {
    #[error("the sandbox tool secret {reference} could not be read")]
    Secret {
        reference: String,
        #[source]
        source: SecretLookupError,
    },
}

#[derive(Debug, Clone)]
pub struct SandboxTool {
    pub provider: String,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SandboxToolRegistry {
    tools: BTreeMap<String, SandboxTool>,
}

impl SandboxToolRegistry {
    pub fn register(
        &mut self,
        entries: &[SandboxToolEntry],
        lookup: &SecretLookup,
        environment: &BTreeMap<String, String>,
    ) -> Result<(), SandboxRegistryError> {
        let mut tools = BTreeMap::new();
        for entry in entries {
            let SandboxToolEntry::Configured(config) = entry else {
                tracing::warn!("sandbox_tools: skipping non-dict entry");
                continue;
            };
            let Some(name) = config.name.as_deref().filter(|name| !name.is_empty()) else {
                tracing::warn!("sandbox_tools: skipping entry missing 'sandbox_tool_name'");
                continue;
            };
            let Some(provider) = config
                .provider
                .as_deref()
                .filter(|provider| !provider.is_empty())
            else {
                tracing::warn!("sandbox_tools: skipping entry missing 'sandbox_provider'");
                continue;
            };
            let api_key = resolve_value(config.api_key.as_deref(), lookup, environment)?;
            let api_base = resolve_value(config.api_base.as_deref(), lookup, environment)?
                .map(|secret| secret.expose_secret().to_owned());
            tools.insert(
                name.to_owned(),
                SandboxTool {
                    provider: provider.to_owned(),
                    api_key,
                    api_base,
                },
            );
        }
        self.tools = tools;
        Ok(())
    }

    pub fn resolve(&self, name: &str) -> Option<&SandboxTool> {
        self.tools.get(name)
    }
}

fn resolve_value(
    value: Option<&str>,
    lookup: &SecretLookup,
    environment: &BTreeMap<String, String>,
) -> Result<Option<SecretString>, SandboxRegistryError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.starts_with(OS_ENVIRON_PREFIX) {
        return lookup
            .get_secret_str(environment, value, None)
            .map_err(|source| SandboxRegistryError::Secret {
                reference: value.to_owned(),
                source,
            });
    }
    Ok(Some(SecretString::from(value)))
}
