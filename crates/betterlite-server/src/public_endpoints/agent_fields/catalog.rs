use std::sync::Arc;

use super::error::AgentFieldsError;
use super::types::{AgentCreateInfo, AgentCredentialField, RawAgentCreateInfo};
use crate::public_endpoints::providers::catalog::provider_catalog;
use crate::public_endpoints::providers::types::{ProviderCreateInfo, ProviderCredentialField};

const MAX_CATALOG_BYTES: usize = 512 * 1024;
const AGENT_CATALOG_ASSET: &[u8] = include_bytes!("../../../data/agent_create_fields.json");

pub fn raw_agent_catalog() -> Result<Vec<RawAgentCreateInfo>, AgentFieldsError> {
    if AGENT_CATALOG_ASSET.len() > MAX_CATALOG_BYTES {
        return Err(AgentFieldsError::CatalogTooLarge {
            size: AGENT_CATALOG_ASSET.len(),
            limit: MAX_CATALOG_BYTES,
        });
    }
    serde_json::from_slice(AGENT_CATALOG_ASSET)
        .map_err(|error| AgentFieldsError::CatalogJson(Arc::new(error)))
}

fn inherited_field(field: ProviderCredentialField) -> AgentCredentialField {
    AgentCredentialField {
        key: field.key,
        label: field.label,
        placeholder: field.placeholder,
        tooltip: field.tooltip,
        required: field.required,
        field_type: field.field_type,
        options: field.options,
        default_value: field.default_value,
        include_in_litellm_params: Some(true),
        validation_pattern: None,
        validation_message: None,
    }
}

fn merge_inherited_credentials(
    raw: RawAgentCreateInfo,
    providers: &[ProviderCreateInfo],
) -> AgentCreateInfo {
    let mut credential_fields = raw.credential_fields;
    if let Some(provider_name) = &raw.inherit_from_provider
        && let Some(provider) = providers
            .iter()
            .find(|entry| &entry.provider == provider_name)
    {
        credential_fields.extend(
            provider
                .credential_fields
                .iter()
                .cloned()
                .map(inherited_field),
        );
    }
    AgentCreateInfo {
        agent_type: raw.agent_type,
        agent_type_display_name: raw.agent_type_display_name,
        description: raw.description,
        logo_url: raw.logo_url,
        credential_fields,
        litellm_params_template: raw.litellm_params_template,
        model_template: raw.model_template,
    }
}

pub fn agent_fields() -> Result<Vec<AgentCreateInfo>, AgentFieldsError> {
    let raw_agents = raw_agent_catalog()?;
    let providers = provider_catalog().map_err(AgentFieldsError::Providers)?;
    Ok(raw_agents
        .into_iter()
        .map(|raw| merge_inherited_credentials(raw, &providers))
        .collect())
}
