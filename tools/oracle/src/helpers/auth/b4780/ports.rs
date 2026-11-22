use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, ready};
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

// a case port that cannot answer raises the same 500 a broken real port would
const fn case_failure(detail: String) -> AgentAccessError {
    AgentAccessError::Rejected {
        status: 500,
        detail,
    }
}

fn holder(auth: &UserAPIKeyAuth) -> Result<Holder, AgentAccessError> {
    serde_json::to_string(auth)
        .and_then(|text| serde_json::from_str(&text))
        .map_err(|error| case_failure(error.to_string()))
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
    fn key_team_access(
        &self,
        auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        let Some(given) = auth else {
            return ready(Ok(AgentAccess::Unrestricted));
        };
        let granted = holder(given).and_then(|found| {
            self.access
                .get(&found)
                .cloned()
                .ok_or_else(|| case_failure(format!("the case grants nothing to {found:?}")))
        });
        ready(granted)
    }

    fn caller_access(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        ready(Ok(AgentAccess::Unrestricted))
    }

    fn key_access_strict(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<AgentAccess, AgentAccessError>> + Send {
        ready(Ok(AgentAccess::Unrestricted))
    }

    fn access_group_ceiling(
        &self,
        _auth: Option<&UserAPIKeyAuth>,
        _strict: bool,
    ) -> impl Future<Output = Result<Option<BTreeSet<String>>, AgentAccessError>> + Send {
        ready(Ok(None))
    }

    fn managed_ceilings(
        &self,
        _agent_id: &str,
        _access_group_ids: &[String],
    ) -> impl Future<Output = Result<Vec<BTreeSet<String>>, AgentAccessError>> + Send {
        ready(Ok(Vec::new()))
    }
}

pub(super) struct NoTargets;

impl TargetAgents for NoTargets {
    fn has_database(&self) -> bool {
        false
    }

    fn agent(
        &self,
        _agent_id: &str,
    ) -> impl Future<Output = Result<Option<AgentResponse>, AgentIdentityFailure>> + Send {
        ready(Ok(None))
    }
}

// LiteLLM's effective_contexts, reload_admitted_user and admitted_subject_sources
pub(super) struct CaseHumans<'case> {
    pub(super) contexts: Vec<UserAPIKeyAuth>,
    pub(super) sources: &'case [SourceFields],
}

impl HumanSources for CaseHumans<'_> {
    fn reload_admitted_key(
        &self,
        _key_hash: &SecretString,
        _check_db_only: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send {
        ready(Err(case_failure(String::from("the cases reload no key"))))
    }

    fn reload_admitted_user(
        &self,
        user_id: &str,
        requires_fresh_policy: bool,
    ) -> impl Future<Output = Result<UserAPIKeyAuth, AgentAccessError>> + Send {
        if !requires_fresh_policy {
            return ready(Err(case_failure(String::from(
                "the human is reloaded without a fresh policy",
            ))));
        }
        ready(key(Some(user_id), None).map_err(|error| case_failure(error.to_string())))
    }

    fn admitted_subject_sources(
        &self,
        human: &UserAPIKeyAuth,
        allowed_team_ids: Option<&BTreeSet<String>>,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, AgentAccessError>> + Send {
        ready(holder(human).and_then(|found| {
            let user_id = found.user_id;
            self.sources
                .iter()
                .filter(|source| {
                    source.team_id.as_ref().is_none_or(|team_id| {
                        allowed_team_ids.is_none_or(|allowed| allowed.contains(team_id))
                    })
                })
                .map(|source| key(user_id.as_deref(), source.team_id.as_deref()))
                .collect::<Result<Vec<UserAPIKeyAuth>, AuthError>>()
                .map_err(|error| case_failure(error.to_string()))
        }))
    }

    fn effective_contexts(
        &self,
        _auth: &UserAPIKeyAuth,
    ) -> impl Future<Output = Result<Vec<UserAPIKeyAuth>, AgentAccessError>> + Send {
        ready(Ok(self.contexts.clone()))
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
