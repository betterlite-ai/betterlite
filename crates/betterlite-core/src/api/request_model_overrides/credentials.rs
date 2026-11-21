use betterlite_models::model_registration::JsonObject;

use crate::api::request_destination::RequestData;
use crate::api::request_model_overrides::diagnostics::OverrideDiagnostic;
use crate::api::request_model_overrides::model_config::{DEFAULT_ENTRY_NAME, ModelConfig};

// the three request fields a credential may replace, in the order LiteLLM copies them
pub(crate) const CREDENTIAL_FIELDS: [&str; 3] = ["api_base", "api_key", "api_version"];

pub(crate) struct CredentialLookup<'request> {
    pub(crate) model: &'request str,
    pub(crate) pre_alias_model: Option<&'request str>,
    pub(crate) provider: Option<&'request str>,
    pub(crate) project: Option<&'request ModelConfig>,
    pub(crate) team: Option<&'request ModelConfig>,
}

// _resolve_credential_from_model_config: project model, pre-alias, defaultconfig, then the team chain
pub(crate) fn resolve_credential(
    lookup: &CredentialLookup<'_>,
    diagnostics: &mut Vec<OverrideDiagnostic>,
) -> Option<String> {
    let mut names = vec![lookup.model];
    if let Some(pre_alias) = lookup
        .pre_alias_model
        .filter(|pre_alias| *pre_alias != lookup.model)
    {
        names.push(pre_alias);
    }
    for config in [lookup.project, lookup.team].into_iter().flatten() {
        for name in &names {
            let Some(entry) = config.entry(name) else {
                continue;
            };
            if let Some(credential) = entry.credential(lookup.provider) {
                return Some(credential);
            }
            diagnostics.push(OverrideDiagnostic::ModelEntryWithoutCredentials {
                model: (*name).to_owned(),
            });
        }
        if let Some(credential) = config
            .entry(DEFAULT_ENTRY_NAME)
            .and_then(|entry| entry.credential(lookup.provider))
        {
            return Some(credential);
        }
    }
    None
}

// _apply_credential_overrides_from_model_config copies only the fields the client did not send
pub(crate) fn apply_credential_values(data: &mut RequestData, values: &JsonObject) {
    for field in CREDENTIAL_FIELDS {
        if let Some(value) = values.get(field)
            && !data.contains_key(field)
        {
            data.insert(field.to_owned(), value.clone());
        }
    }
}
