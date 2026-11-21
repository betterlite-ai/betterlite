use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use betterlite_auth::agent_access::error::AgentAccessError;
use betterlite_auth::agent_access::reads::{GrantSources, HumanSources, TargetAgents};
use betterlite_auth::agent_access::resolver::AgentAccessResolver;
use betterlite_auth::agent_access_groups::access::AgentAccess;
use betterlite_auth::agent_policy::AgentResponse;
use betterlite_auth::agent_registry::AgentRegistry;
use betterlite_auth::identity::AgentIdentityFailure;
use betterlite_auth::resolved_key::auth::{AuthError, AuthInput, UserAPIKeyAuth};
use betterlite_db::budget_permission_rows::JsonData;
use secrecy::SecretString;
use serde::Deserialize;

use super::SourceFields;

// the user and team a fake port reads back from an auth, whose fields betterlite-auth keeps private
#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Holder {
    pub(super) user_id: Option<String>,
    pub(super) team_id: Option<String>,
}

fn holder(auth: &UserAPIKeyAuth) -> Result<Holder, AgentAccessError> {
    serde_json::to_string(auth)
        .and_then(|text| serde_json::from_str(&text))
        .map_err(|error| AgentAccessError::Rejected {
            status: 500,
            detail: error.to_string(),
        })
}

pub(super) fn key(
    user_id: Option<&str>,
    team_id: Option<&str>,
) -> Result<UserAPIKeyAuth, AuthError> {
    let mut fields = BTreeMap::new();
    for (name, value) in [("user_id", user_id), ("team_id", team_id)] {
        if let Some(text) = value {
            fields.insert(name.to_owned(), JsonData::String(text.to_owned()));
        }
    }
    UserAPIKeyAuth::construct(AuthInput::Dict(fields))
}

// LiteLLM's resolve_access, and for a human source its key and team grants: Restricted, so strict mode keeps it
pub(super) struct CaseGrants {
    pub(super) access: BTreeMap<Holder, AgentAccess>,
}

impl GrantSources for CaseGrants {
    async fn key_team_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> Result<AgentAccess, AgentAccessError> {
        let Some(given) = auth else {
            return Ok(AgentAccess::Unrestricted);
        };
        let found = holder(given)?;
        self.access
            .get(&found)
            .cloned()
            .ok_or_else(|| AgentAccessError::Rejected {
                status: 500,
                detail: format!("the case grants nothing to {found:?}"),
            })
    }

    async fn caller_access(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> Result<AgentAccess, AgentAccessError> {
        Ok(AgentAccess::Unrestricted)
    }

    async fn key_access_strict(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<AgentAccess, AgentAccessError> {
        Ok(AgentAccess::Unrestricted)
    }

    async fn access_group_ceiling(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> Result<Option<BTreeSet<String>>, AgentAccessError> {
        Ok(None)
    }

    async fn managed_ceilings(
        &self,
        _agent_id: &str,
        _access_group_ids: &[String],
    ) -> Result<Vec<BTreeSet<String>>, AgentAccessError> {
        Ok(Vec::new())
    }
}

pub(super) struct NoTargets;

impl TargetAgents for NoTargets {
    fn has_database(&self) -> bool {
        false
    }

    async fn agent(&self, _agent_id: &str) -> Result<Option<AgentResponse>, AgentIdentityFailure> {
        Ok(None)
    }
}

// LiteLLM's effective_contexts, reload_admitted_user and admitted_subject_sources
pub(super) struct CaseHumans<'case> {
    pub(super) contexts: Vec<UserAPIKeyAuth>,
    pub(super) sources: &'case [SourceFields],
}

impl HumanSources for CaseHumans<'_> {
    async fn reload_admitted_key(
        &self,
        _key_hash: &SecretString,
        _check_db_only: bool,
    ) -> Result<UserAPIKeyAuth, AgentAccessError> {
        Err(AgentAccessError::Rejected {
            status: 500,
            detail: String::from("the cases reload no key"),
        })
    }

    async fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> Result<UserAPIKeyAuth, AgentAccessError> {
        if !requires_fresh_policy {
            return Err(AgentAccessError::Rejected {
                status: 500,
                detail: String::from("the human is reloaded without a fresh policy"),
            });
        }
        key(Some(user_id), None).map_err(|error| AgentAccessError::Rejected {
            status: 500,
            detail: error.to_string(),
        })
    }

    async fn admitted_subject_sources(
        &self,
        human: &UserAPIKeyAuth,
        allowed_team_ids: Option<&BTreeSet<String>>,
    ) -> Result<Vec<UserAPIKeyAuth>, AgentAccessError> {
        let user_id = holder(human)?.user_id;
        let admitted = self.sources.iter().filter(|source| {
            source.team_id.as_ref().is_none_or(|team_id| {
                allowed_team_ids.is_none_or(|allowed| allowed.contains(team_id))
            })
        });
        admitted
            .map(|source| key(user_id.as_deref(), source.team_id.as_deref()))
            .collect::<Result<Vec<UserAPIKeyAuth>, AuthError>>()
            .map_err(|error| AgentAccessError::Rejected {
                status: 500,
                detail: error.to_string(),
            })
    }

    async fn effective_contexts(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> Result<Vec<UserAPIKeyAuth>, AgentAccessError> {
        Ok(self.contexts.clone())
    }
}

pub(super) type CaseResolver<'case> = AgentAccessResolver<CaseGrants, NoTargets, CaseHumans<'case>>;

// each source of the human answers with its own agent ids
pub(super) fn source_resolver<'case>(
    user_id: Option<&str>,
    sources: &'case [SourceFields],
) -> CaseResolver<'case> {
    let access = sources
        .iter()
        .map(|source| {
            let source_holder = Holder {
                user_id: user_id.map(str::to_owned),
                team_id: source.team_id.clone(),
            };
            (
                source_holder,
                AgentAccess::Restricted(source.agent_ids.clone()),
            )
        })
        .collect();
    AgentAccessResolver {
        registry: Arc::new(AgentRegistry::default()),
        grants: CaseGrants { access },
        targets: NoTargets,
        humans: CaseHumans {
            contexts: Vec::new(),
            sources,
        },
    }
}
