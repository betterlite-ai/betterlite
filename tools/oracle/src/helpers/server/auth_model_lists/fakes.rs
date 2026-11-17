use std::fmt::{Debug, Display};
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_auth::auth_model_lists::credential::{CredentialEntry, CredentialSnapshot};
use betterlite_auth::auth_model_lists::fallbacks::{FallbackError, FallbackGroups};
use betterlite_auth::auth_model_lists::parameters::ModelParameters;
use betterlite_auth::auth_model_lists::providers::{ProviderModels, is_declared_provider};
use betterlite_auth::auth_model_lists::wildcard::{DeploymentLookup, WildcardError};
use betterlite_config::sections::fallback_plans::chains::{FallbackChainEntry, FallbackTarget};
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use indexmap::IndexMap;
use serde::Deserialize;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};
#[derive(Deserialize)]
pub(crate) struct Credential {
    credential_name: String,
    credential_values: JsonObject,
}
pub(crate) fn snapshot(credentials: &[Credential]) -> CredentialSnapshot {
    CredentialSnapshot::new(
        credentials
            .iter()
            .map(|credential| CredentialEntry {
                credential_name: credential.credential_name.clone(),
                credential_values: credential.credential_values.clone(),
            })
            .collect(),
    )
}
#[derive(Deserialize)]
pub(crate) struct Row {
    model_name: String,
    litellm_params: JsonObject,
    team_id: Option<String>,
    team_public_model_name: Option<String>,
}
// B2328's enumeration, injected: the key joins the provider with the credential values the lookup received.
pub(crate) struct Catalog<'data>(pub(crate) &'data IndexMap<String, Vec<String>>);
impl ProviderModels for Catalog<'_> {
    fn valid_models(
        &self,
        provider: Option<&str>,
        parameters: Option<&ModelParameters>,
    ) -> Option<Vec<String>> {
        let Some(name) = provider else {
            return Some(Vec::new());
        };
        // LiteLLM answers for a provider outside models_by_provider without asking the enumerator
        if !is_declared_provider(name) {
            return None;
        }
        let key = parameters.map_or_else(
            || name.to_owned(),
            |found| {
                format!(
                    "{name}|{}|{}|{}",
                    found.text("api_key").unwrap_or("None"),
                    found.text("api_base").unwrap_or("None"),
                    found.text("litellm_credential_name").unwrap_or("None"),
                )
            },
        );
        Some(self.0.get(&key).cloned().unwrap_or_default())
    }
}
pub(crate) struct Rows<'data>(pub(crate) &'data [Row]);
impl DeploymentLookup for Rows<'_> {
    fn model_rows(
        &self,
        model: &str,
        team_id: Option<&str>,
    ) -> Result<Vec<ModelParameters>, WildcardError> {
        let mut parameters = Vec::new();
        for row in self.0 {
            let team_public = row.team_id.is_some()
                && team_id == row.team_id.as_deref()
                && row.team_public_model_name.as_deref() == Some(model);
            let deployed = row.model_name == model
                && (team_id.is_none()
                    || row.team_id.is_none()
                    || team_id == row.team_id.as_deref());
            if team_public || deployed {
                parameters.push(ModelParameters::parse(&row.litellm_params)?);
            }
        }
        Ok(parameters)
    }
}
#[derive(Deserialize)]
pub(crate) struct FallbackSettings {
    pub(crate) fallbacks: Vec<FallbackChainEntry>,
    pub(crate) context_window_fallbacks: Vec<FallbackChainEntry>,
    pub(crate) content_policy_fallbacks: Vec<FallbackChainEntry>,
}
// B309's group resolution for the two shapes the cases configure: an exact key, then the "*" chain.
pub(crate) struct Resolver;
impl FallbackGroups for Resolver {
    fn fallback_group(
        &self,
        chain: &[FallbackChainEntry],
        model: &str,
    ) -> Result<Option<Vec<String>>, FallbackError> {
        let mut generic = None;
        for entry in chain {
            let FallbackChainEntry::Group(groups) = entry else {
                continue;
            };
            let Some((name, targets)) = groups.iter().next() else {
                continue;
            };
            if name == model {
                return Ok(Some(names(targets)));
            }
            if name == "*" {
                generic = Some(targets);
            }
        }
        Ok(generic.map(|targets| names(targets)))
    }
}
fn names(targets: &[FallbackTarget]) -> Vec<String> {
    targets
        .iter()
        .map(|target| match target {
            FallbackTarget::Model(name) => name.clone(),
            FallbackTarget::Parameters(parameters) => parameters.model.clone(),
        })
        .collect()
}
#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<String>>>);
#[derive(Default)]
struct Message(String);
impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}
impl<Inner: Subscriber> Layer<Inner> for Logs {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, Inner>) {
        if *event.metadata().level() != Level::WARN {
            return;
        }
        let mut message = Message::default();
        event.record(&mut message);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.0);
    }
}
pub(crate) fn captured<T>(run: &dyn Fn() -> T) -> (T, Vec<String>) {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::registry().with(logs.clone());
    let value = with_default(subscriber, run);
    let warnings = logs
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    (value, warnings)
}
pub(crate) fn failure(error: &dyn Display) -> Box<HelperError> {
    Box::new(HelperError {
        class: "ModelListError".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: error.to_string(),
    })
}
