use betterlite_config::sections::guardrail_params::Settings;
use betterlite_config::sections::guardrail_params::records::GuardrailDefinitionLocation;

use super::error::StartupError;
use super::router_list::{RouterGuardrail, router_guardrail_list};
use super::runtime::GuardrailRuntime;
use crate::guardrail_registry::error::RegistryError;
use crate::guardrail_registry::rows::GuardrailRows;

// LiteLLM catches only ValueError and TypeError here; the hide-secrets enterprise error is a plain Exception
const fn skips_startup(error: &RegistryError) -> bool {
    match error {
        RegistryError::Unsupported { .. }
        | RegistryError::MissingGuardrailType
        | RegistryError::ScanOnlyUnsupported { .. }
        | RegistryError::ScanOnlyWithSkipTool { .. }
        | RegistryError::Base(_)
        | RegistryError::Initializer(_)
        | RegistryError::InitFailed { .. } => true,
        RegistryError::Enterprise(_)
        | RegistryError::Rows(_)
        | RegistryError::Params(_)
        | RegistryError::Row { .. }
        | RegistryError::Encode { .. }
        | RegistryError::Decode { .. }
        | RegistryError::MissingParams
        | RegistryError::UpdateNotFound { .. }
        | RegistryError::Database { .. } => false,
    }
}

// a free function, since the workspace denies a second inherent impl of GuardrailRuntime
pub async fn start_from_config<R: GuardrailRows>(
    runtime: &GuardrailRuntime<R>,
    settings: &Settings,
) -> Result<Vec<RouterGuardrail>, StartupError> {
    for rejected in &settings.rejected {
        tracing::error!(
            "Skipping guardrail '{}': invalid configuration, proxy is starting WITHOUT this guardrail: {}",
            rejected.guardrail_name,
            rejected.reason
        );
    }
    let mut initialized = Vec::new();
    for guardrail in &settings.guardrails {
        let outcome = runtime
            .handler
            .write()
            .await
            .initialize_guardrail(guardrail.clone(), GuardrailDefinitionLocation::Config);
        match outcome {
            Ok(held) => initialized.push(held),
            Err(error) if skips_startup(&error) => {
                tracing::error!(
                    "Skipping guardrail '{}': invalid configuration, proxy is starting WITHOUT this guardrail: {error}",
                    guardrail.guardrail_name
                );
            }
            Err(source) => {
                return Err(StartupError::Guardrail {
                    guardrail_name: guardrail.guardrail_name.clone(),
                    source,
                });
            }
        }
    }
    Ok(router_guardrail_list(&initialized))
}
