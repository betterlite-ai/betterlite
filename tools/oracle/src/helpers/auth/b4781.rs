use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_auth::agent_access::resolver::AgentAccessResolver;
use betterlite_auth::agent_policy::{AgentResponse, ManagedAgentContext};
use betterlite_auth::agent_registry::{AgentRegistry, AgentRegistryApi as _};
use betterlite_auth::managed_admission::admit::admit_managed_actor;
use betterlite_auth::managed_admission::error::AdmissionError;
use betterlite_auth::managed_admission::invoke::prepare_agent_invocation;
use betterlite_auth::managed_admission::ports::InvocationSources;
use betterlite_auth::managed_admission::{managed_marks, managed_token};
use betterlite_auth::request_quota_view::RequestQuotaViewExt as _;
use betterlite_auth::resolved_key::auth::UserAPIKeyAuth;
use betterlite_db::budget_permission_rows::JsonData;
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use ports::{CaseGrants, CaseHumans, CaseResolver, CaseStore, CaseTargets, NoReadThrough};

mod ports;
mod tests;

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    Admit {
        auth: AuthFields,
        registry: Vec<String>,
        store: Option<Vec<String>>,
        retired: bool,
        granted: Vec<String>,
    },
    Invoke {
        auth: AuthFields,
        target_name: String,
        registry: Vec<String>,
        store: Option<Vec<String>>,
        granted: Vec<String>,
        billable: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AuthFields {
    user_id: Option<String>,
    team_id: Option<String>,
    agent_id: Option<String>,
    context: Option<ManagedAgentContext>,
}

// the shared inputs of both functions: the token, the registry rows, the store rows and the human's grants
pub(crate) struct Case<'case> {
    auth: &'case AuthFields,
    registry: &'case [String],
    store: Option<&'case [String]>,
    granted: &'case [String],
}

impl<'case> Case<'case> {
    fn of(input: &'case Input) -> Self {
        let (auth, registry, store, granted) = match input {
            Input::Admit {
                auth,
                registry,
                store,
                granted,
                ..
            }
            | Input::Invoke {
                auth,
                registry,
                store,
                granted,
                ..
            } => (auth, registry, store, granted),
        };
        Self {
            auth,
            registry: registry.as_slice(),
            store: store.as_deref(),
            granted: granted.as_slice(),
        }
    }
}

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Input) -> HelperResult {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let case = Case::of(input);
        match input {
            Input::Admit { retired, .. } => admit_case(&runtime, &case, *retired),
            Input::Invoke {
                target_name,
                billable,
                ..
            } => invoke_case(&runtime, &case, target_name, *billable),
        }
    }
}

fn admit_case(runtime: &Runtime, case: &Case<'_>, retired: bool) -> HelperResult {
    let registry = registry(case.registry)?;
    let rows = store_rows(case.store)?;
    let store = rows.map(|agents| CaseStore { agents, retired });
    let access = access(&registry, store.as_ref(), case.granted);
    let mut auth = token(case.auth)?;
    let outcome = runtime.block_on(admit_managed_actor(
        &mut auth,
        store.as_ref(),
        &registry,
        &access,
    ));
    answer(&auth, outcome)
}

fn invoke_case(
    runtime: &Runtime,
    case: &Case<'_>,
    target_name: &str,
    billable: bool,
) -> HelperResult {
    let registry = registry(case.registry)?;
    let rows = store_rows(case.store)?;
    let store = rows.map(|agents| CaseStore {
        agents,
        retired: false,
    });
    let access = access(&registry, store.as_ref(), case.granted);
    let mut auth = token(case.auth)?;
    let sources = InvocationSources {
        store: store.as_ref(),
        read_through: &NoReadThrough,
    };
    let outcome = runtime.block_on(prepare_agent_invocation(
        &mut auth,
        target_name,
        billable,
        &sources,
        &access,
    ));
    answer(&auth, outcome)
}

fn registry(rows: &[String]) -> Result<Arc<AgentRegistry>, Box<HelperError>> {
    let registry = AgentRegistry::default();
    for row in rows {
        registry.register_agent(serde_json::from_str(row).or_value_error()?);
    }
    Ok(Arc::new(registry))
}

fn store_rows(rows: Option<&[String]>) -> Result<Option<Vec<AgentResponse>>, Box<HelperError>> {
    let Some(list) = rows else {
        return Ok(None);
    };
    let mut parsed = Vec::with_capacity(list.len());
    for row in list {
        parsed.push(serde_json::from_str(row).or_value_error()?);
    }
    Ok(Some(parsed))
}

fn access(
    registry: &Arc<AgentRegistry>,
    store: Option<&CaseStore>,
    granted: &[String],
) -> CaseResolver {
    AgentAccessResolver {
        registry: Arc::clone(registry),
        grants: CaseGrants {
            granted: granted.iter().cloned().collect(),
        },
        targets: CaseTargets {
            store: store.map_or(Vec::new(), |case| case.agents.clone()),
            registry: Arc::clone(registry),
            database: store.is_some(),
        },
        humans: CaseHumans,
    }
}

fn token(fields: &AuthFields) -> Result<UserAPIKeyAuth, Box<HelperError>> {
    let mut decoded = BTreeMap::new();
    for (name, value) in [
        ("user_id", fields.user_id.as_deref()),
        ("team_id", fields.team_id.as_deref()),
        ("agent_id", fields.agent_id.as_deref()),
    ] {
        if let Some(text) = value {
            decoded.insert(name.to_owned(), JsonData::String(text.to_owned()));
        }
    }
    managed_token(decoded, fields.context.clone()).or_value_error()
}

// the marks the auth carries after the call, whether it passed or failed, as the case output compares them
#[derive(Serialize)]
struct Answer {
    outcome: Verdict,
    managed_agent_policy: Option<String>,
    billing_agent_policy: Option<String>,
    requires_fresh_policy: bool,
    invoked_agent_id: Option<String>,
    invoked_agent_policy: Option<String>,
    agent_invocation_cost: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Verdict {
    Ok(&'static str),
    Refused { status: u16, detail: String },
}

fn answer(auth: &UserAPIKeyAuth, outcome: Result<(), AdmissionError>) -> HelperResult {
    let verdict = match outcome {
        Ok(()) => Verdict::Ok("ok"),
        Err(error) => Verdict::Refused {
            status: error.status,
            detail: error.detail,
        },
    };
    let marks = managed_marks(auth);
    let view = auth.request_quota_view();
    let answer = Answer {
        outcome: verdict,
        managed_agent_policy: view
            .managed_agent_policy
            .map(|agent| agent.agent_id.clone()),
        billing_agent_policy: marks.billing_agent_policy.map(|agent| agent.agent_id),
        requires_fresh_policy: marks.requires_fresh_policy,
        invoked_agent_id: view.invoked_agent_id.map(str::to_owned),
        invoked_agent_policy: view
            .invoked_agent_policy
            .map(|agent| agent.agent_id.clone()),
        agent_invocation_cost: marks
            .agent_invocation_cost
            .map(|cost| cost.normalize().to_string()),
    };
    serde_json::to_value(answer).or_value_error()
}
