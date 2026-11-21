use std::collections::BTreeSet;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_logging::callback_config::CallbackVars;
use betterlite_logging::callback_config::parse::{CallbackType, TeamCallback};
use betterlite_logging::otel::config::destination::OtelDestination;
use betterlite_logging::otel::presets::destinations::destination_for;
use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::StandardCallbackDynamicParams;
use indexmap::{IndexMap, IndexSet};

use crate::api::request_principal_metadata::{Metadata, PrincipalFacts};
use crate::api::request_setup_options::callbacks::{logging_entries, validated_callbacks};
use crate::api::request_setup_options::{
    CallbackSource, CallbackVarsDecryption, RequestSetupOptions, SetupDiagnostic,
};

const DISABLE_CALLBACKS_HEADER: &str = "x-litellm-disable-callbacks";
const DISABLED_CALLBACKS_KEY: &str = "litellm_disabled_callbacks";
const SERVICE_NAME_KEYS: [&str; 2] = ["otel_service_name_override", "otel_service_name"];
const SAMPLING_RATE_PARAMS: [&str; 3] = [
    "langsmith_sampling_rate",
    "arize_success_sampling_rate",
    "arize_error_sampling_rate",
];

#[derive(Debug, Clone, Default)]
pub struct TenantDestinations {
    pub destinations: Vec<OtelDestination>,
    pub diagnostics: Vec<SetupDiagnostic>,
}

// the header wins over the key's stored list and team metadata is never a source, the way callback dispatch reads it
#[must_use]
pub fn disabled_backends(
    allowed: bool,
    key_metadata: &Metadata,
    headers: &IndexMap<String, String>,
) -> BTreeSet<String> {
    if !allowed {
        return BTreeSet::new();
    }
    if let Some((_, header)) = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(DISABLE_CALLBACKS_HEADER))
    {
        return header
            .split(',')
            .map(|name| name.trim().to_lowercase())
            .collect();
    }
    let Some(Json::List(names)) = key_metadata.get(DISABLED_CALLBACKS_KEY) else {
        return BTreeSet::new();
    };
    names
        .iter()
        .filter_map(|name| {
            if let Json::Text(text) = name {
                Some(text.to_lowercase())
            } else {
                None
            }
        })
        .collect()
}

impl<Ids, Decryption, Environment> RequestSetupOptions<Ids, Decryption, Environment>
where
    Decryption: CallbackVarsDecryption,
    Environment: EnvironmentLookup,
{
    // resolve_tenant_otel_destinations: the key's entries win over the team's outright, even an empty list
    #[must_use]
    pub fn tenant_destinations(
        &self,
        principal: &PrincipalFacts,
        headers: &IndexMap<String, String>,
    ) -> TenantDestinations {
        if !self.settings.otel_v2_enabled {
            return TenantDestinations::default();
        }
        let entries = logging_entries(&principal.metadata, &self.decryption).or_else(|| {
            principal
                .team_metadata
                .as_ref()
                .and_then(|metadata| logging_entries(metadata, &self.decryption))
        });
        let Some(found) =
            entries.filter(|found| !matches!(found, Json::List(items) if items.is_empty()))
        else {
            return TenantDestinations::default();
        };
        let disabled = disabled_backends(
            self.settings.callback_disabling,
            &principal.metadata,
            headers,
        );
        let mut diagnostics = Vec::new();
        let callbacks: Vec<TeamCallback> =
            validated_callbacks(&found, CallbackSource::OtelDestination, &mut diagnostics)
                .into_iter()
                .filter(|callback| !disabled.contains(&callback.callback_name.to_lowercase()))
                .collect();
        TenantDestinations {
            destinations: self.destinations(&callbacks, principal),
            diagnostics,
        }
    }

    // failure-only entries name no destination, yet their variables still merge into a backend another entry names
    fn destinations(
        &self,
        callbacks: &[TeamCallback],
        principal: &PrincipalFacts,
    ) -> Vec<OtelDestination> {
        let names: IndexSet<&str> = callbacks
            .iter()
            .filter(|callback| callback.callback_type != CallbackType::Failure)
            .map(|callback| callback.callback_name.as_str())
            .collect();
        let service_name = tenant_service_name(principal);
        names
            .into_iter()
            .filter_map(|name| {
                let mut vars = CallbackVars::new();
                for callback in callbacks
                    .iter()
                    .filter(|callback| callback.callback_name == name)
                {
                    vars.extend(callback.callback_vars.clone());
                }
                destination_for(
                    &self.environment,
                    &self.settings.allowed_hosts,
                    name,
                    &tenant_params(&vars),
                    service_name.as_deref(),
                )
            })
            .collect()
    }
}

// the key's own service name wins over every team field, and a blank one counts as unset
fn tenant_service_name(principal: &PrincipalFacts) -> Option<String> {
    [Some(&principal.metadata), principal.team_metadata.as_ref()]
        .into_iter()
        .flatten()
        .flat_map(|source| {
            SERVICE_NAME_KEYS.iter().filter_map(move |key| {
                if let Some(Json::Text(value)) = source.get(*key) {
                    Some(value.trim())
                } else {
                    None
                }
            })
        })
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

// _tenant_otel_params: one value StandardCallbackDynamicParams can't hold empties the whole map, so the operator's exporters stay
fn tenant_params(vars: &CallbackVars) -> StandardCallbackDynamicParams {
    let fits = vars.iter().all(|(name, value)| match name.as_str() {
        "langfuse_prompt_version" => value.trim().parse::<i64>().is_ok(),
        "turn_off_message_logging" => {
            betterlite_env::flag(name, value).is_ok_and(|parsed| parsed.is_some())
        }
        "litellm_disabled_callbacks" => false,
        // pydantic's float takes what f64 parsing takes, inf and nan included
        other if SAMPLING_RATE_PARAMS.contains(&other) => value.trim().parse::<f64>().is_ok(),
        _ => true,
    });
    if !fits {
        return Json::Object(IndexMap::new());
    }
    Json::Object(
        vars.iter()
            .map(|(name, value)| (name.clone(), Json::Text(value.clone())))
            .collect(),
    )
}
