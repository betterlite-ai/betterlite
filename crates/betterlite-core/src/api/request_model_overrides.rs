pub mod catalog;
pub mod diagnostics;
pub mod error;

pub(crate) mod alias;
pub(crate) mod credentials;
pub(crate) mod enforced;
pub(crate) mod model_config;
pub(crate) mod warnings;

mod tests;

use std::fmt;
use std::sync::Arc;

use betterlite_auth::auth_model_lists::credential::CredentialSnapshot;
use betterlite_config::sections::request_model_overrides::Settings as ModelOverridesSettings;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use indexmap::IndexMap;

use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::alias::{AliasInputs, apply_model_aliases};
use crate::api::request_model_overrides::catalog::{DeploymentCatalogView, provider_hint};
use crate::api::request_model_overrides::credentials::{
    CredentialLookup, apply_credential_values, resolve_credential,
};
use crate::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use crate::api::request_model_overrides::enforced::{
    check_enforced_params, resolve_enforced_params,
};
use crate::api::request_model_overrides::error::RequestModelOverridesError;
use crate::api::request_model_overrides::model_config::{MetadataSlot, model_config_from_metadata};
use crate::api::request_model_overrides::warnings::WarnOnce;

pub(crate) const MODEL_FIELD: &str = "model";
const CLIENT_CREDENTIAL_FIELDS: [&str; 2] = ["api_base", "api_key"];

// the fields of UserAPIKeyAuth this slice reads; project_metadata is the project the key belongs to
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OverridesPrincipal {
    pub team_id: Option<String>,
    pub team_model_aliases: IndexMap<String, String>,
    pub aliases: IndexMap<String, String>,
    pub metadata: Option<JsonObject>,
    pub team_metadata: Option<JsonObject>,
    pub project_metadata: Option<JsonObject>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestModelOverridesInput {
    pub data: RequestData,
    pub principal: OverridesPrincipal,
    pub premium_user: bool,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RequestModelOverridesResult {
    pub data: RequestData,
    pub enforced_params: Option<Vec<String>>,
    pub diagnostics: Vec<OverrideDiagnostic>,
}

// the data map holds the credential values this slice copied onto it, so it never reaches a debug line
impl fmt::Debug for RequestModelOverridesResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestModelOverridesResult")
            .field("data", &"<redacted>")
            .field("enforced_params", &self.enforced_params)
            .field("diagnostics", &self.diagnostics)
            .finish()
    }
}

pub struct RequestModelOverridesService {
    settings: ModelOverridesSettings,
    catalog: Option<Arc<dyn DeploymentCatalogView>>,
    credentials: Arc<CredentialSnapshot>,
    stale_alias_warnings: WarnOnce,
}

impl Default for RequestModelOverridesService {
    fn default() -> Self {
        Self {
            settings: ModelOverridesSettings::default(),
            catalog: None,
            credentials: Arc::new(CredentialSnapshot::default()),
            stale_alias_warnings: WarnOnce::default(),
        }
    }
}

impl RequestModelOverridesService {
    #[must_use]
    pub fn new(
        settings: ModelOverridesSettings,
        catalog: Option<Arc<dyn DeploymentCatalogView>>,
        credentials: Arc<CredentialSnapshot>,
    ) -> Self {
        Self {
            settings,
            catalog,
            credentials,
            stale_alias_warnings: WarnOnce::default(),
        }
    }

    // add_litellm_data_to_request's alias, credential and enforced-params stages, in that order
    pub fn apply(
        &self,
        input: &RequestModelOverridesInput,
    ) -> Result<RequestModelOverridesResult, RequestModelOverridesError> {
        let mut data = input.data.clone();
        let pre_alias_model = requested_model(&data)?;
        let mut diagnostics = Vec::new();
        let inputs = AliasInputs {
            catalog: self.catalog.as_deref(),
            team_stale_alias_bypass: self.settings.team_stale_alias_bypass,
            warnings: &self.stale_alias_warnings,
        };
        apply_model_aliases(&mut data, &input.principal, &inputs, &mut diagnostics)?;
        if self.settings.credential_overrides {
            self.apply_credential_override(
                &mut data,
                &input.principal,
                pre_alias_model.as_deref(),
                &mut diagnostics,
            )?;
        }
        let enforced_params = resolve_enforced_params(&self.settings, &input.principal)?;
        check_enforced_params(&data, enforced_params.as_deref(), input.premium_user)?;
        Ok(RequestModelOverridesResult {
            data,
            enforced_params,
            diagnostics,
        })
    }

    fn apply_credential_override(
        &self,
        data: &mut RequestData,
        principal: &OverridesPrincipal,
        pre_alias_model: Option<&str>,
        diagnostics: &mut Vec<OverrideDiagnostic>,
    ) -> Result<(), RequestModelOverridesError> {
        // clientside credentials are the highest precedence, and a null one counts as absent
        let client_credentials = CLIENT_CREDENTIAL_FIELDS.iter().any(|field| {
            data.get(*field)
                .is_some_and(|value| !matches!(value, Json::Null))
        });
        if client_credentials {
            return Ok(());
        }
        let Some(model) = requested_model(data)? else {
            return Ok(());
        };
        let name = self.credential_name(principal, pre_alias_model, &model, diagnostics)?;
        let Some(credential) = name else {
            return Ok(());
        };
        let values = self.credentials.values(&credential);
        if values.is_empty() {
            diagnostics.push(OverrideDiagnostic::CredentialNotFound { credential });
            return Ok(());
        }
        apply_credential_values(data, &values);
        diagnostics.push(OverrideDiagnostic::CredentialApplied { credential, model });
        Ok(())
    }

    // the team and project model_config walks, run only when the feature flag and the client allow it
    fn credential_name(
        &self,
        principal: &OverridesPrincipal,
        pre_alias_model: Option<&str>,
        model: &str,
        diagnostics: &mut Vec<OverrideDiagnostic>,
    ) -> Result<Option<String>, RequestModelOverridesError> {
        let project =
            model_config_from_metadata(principal.project_metadata.as_ref(), MetadataSlot::Project)?;
        let team =
            model_config_from_metadata(principal.team_metadata.as_ref(), MetadataSlot::Team)?;
        if project.is_none() && team.is_none() {
            return Ok(None);
        }
        let provider = provider_hint(model, pre_alias_model, self.catalog.as_deref());
        let lookup = CredentialLookup {
            model,
            pre_alias_model,
            provider: provider.as_deref(),
            project: project.as_ref(),
            team: team.as_ref(),
        };
        Ok(resolve_credential(&lookup, diagnostics))
    }
}

// data.get("model") with LiteLLM's empty-name check; a model of any other type is a field error
pub(crate) fn requested_model(
    data: &RequestData,
) -> Result<Option<String>, RequestModelOverridesError> {
    let Some(value) = data.get(MODEL_FIELD) else {
        return Ok(None);
    };
    match value {
        Json::Null => Ok(None),
        Json::Text(text) if text.is_empty() => Ok(None),
        Json::Text(text) => Ok(Some(text.clone())),
        Json::Bool(_) | Json::Number(_) | Json::List(_) | Json::Object(_) => {
            Err(RequestModelOverridesError::InvalidField {
                field: MODEL_FIELD.to_owned(),
                expected: "a string",
            })
        }
    }
}
