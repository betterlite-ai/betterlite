use uuid::Uuid;

use super::write_fields::{
    IdentityFields, IdentityHistoryEntry, IdentityHistoryKey, IdentityHistoryWrite,
    IdentityRelationWrite, IdentityUpsert, ManagedWriteFields,
};
use crate::agent_policy::{self, AgentResponse};
use crate::identity::EntraIdentityConfig;

const PROVIDER_MICROSOFT_ENTRA: &str = "microsoft_entra";

#[must_use]
pub fn identity_write(
    identity: Option<&EntraIdentityConfig>,
    existing: Option<&AgentResponse>,
    next_revision: fn() -> Uuid,
) -> ManagedWriteFields {
    if let Some(configuration) = identity {
        return bound_identity_fields(configuration, existing, next_revision);
    }
    unbound_identity_fields(existing, next_revision)
}

fn unbound_identity_fields(
    existing: Option<&AgentResponse>,
    next_revision: fn() -> Uuid,
) -> ManagedWriteFields {
    let Some(agent) = existing else {
        return ManagedWriteFields::default();
    };
    let unbind = IdentityFields {
        provider: None,
        tenant_id: None,
        client_id: None,
        issuer: None,
        service_principal_id: None,
        required_roles: None,
        required_scopes: None,
        active: Some(false),
        revision: Some(next_revision().to_string()),
        last_authenticated_at: Some(None),
    };
    ManagedWriteFields {
        enabled: agent.identity_managed.then_some(false),
        execution_mode: None,
        identity_managed: agent.identity_managed.then_some(true),
        identity: agent
            .identity
            .as_ref()
            .map(|_| IdentityRelationWrite::Update(unbind)),
        retired_identities: None,
    }
}

fn bound_identity_fields(
    identity: &EntraIdentityConfig,
    existing: Option<&AgentResponse>,
    next_revision: fn() -> Uuid,
) -> ManagedWriteFields {
    if existing.is_some_and(|agent| binding_unchanged(agent, identity)) {
        return ManagedWriteFields::default();
    }
    let binding = IdentityFields {
        provider: Some(String::from(PROVIDER_MICROSOFT_ENTRA)),
        tenant_id: Some(identity.tenant_id.to_string()),
        client_id: Some(identity.client_id.to_string()),
        issuer: Some(identity.issuer()),
        service_principal_id: Some(identity.service_principal_id.map(|id| id.to_string())),
        required_roles: Some(identity.required_roles.clone()),
        required_scopes: Some(identity.required_scopes.clone()),
        active: Some(true),
        revision: Some(next_revision().to_string()),
        last_authenticated_at: Some(None),
    };
    let relation = if existing.is_some() {
        IdentityRelationWrite::Upsert(Box::new(IdentityUpsert {
            create: binding.clone(),
            update: binding,
        }))
    } else {
        IdentityRelationWrite::Create(binding)
    };
    ManagedWriteFields {
        enabled: None,
        execution_mode: None,
        identity_managed: Some(true),
        identity: Some(relation),
        retired_identities: Some(IdentityHistoryWrite {
            create: IdentityHistoryEntry {
                key: IdentityHistoryKey {
                    provider: String::from(PROVIDER_MICROSOFT_ENTRA),
                    tenant_id: identity.tenant_id.to_string(),
                    client_id: identity.client_id.to_string(),
                },
                issuer: identity.issuer(),
            },
        }),
    }
}

fn binding_unchanged(agent: &AgentResponse, identity: &EntraIdentityConfig) -> bool {
    let Some(binding) = agent.identity.as_ref() else {
        return false;
    };
    binding.active.0
        && binding.provider == agent_policy::IdentityProvider::MicrosoftEntra
        && binding.tenant_id == identity.tenant_id.to_string()
        && binding.client_id == identity.client_id.to_string()
        && binding.service_principal_id == identity.service_principal_id.map(|id| id.to_string())
        && binding.required_roles == identity.required_roles
        && binding.required_scopes.0 == identity.required_scopes
}
