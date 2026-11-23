mod fixtures;

use std::sync::{Arc, PoisonError};

use betterlite_auth::auth_org_lookup::alias::get_org_object_by_alias;
use betterlite_auth::auth_org_lookup::error::OrgLookupError;
use betterlite_auth::auth_org_lookup::last_known::get_org_object_for_request;
use betterlite_auth::auth_org_lookup::organization::{OrgId, OrgLookup, get_org_object};
use betterlite_auth::auth_org_lookup::ports::{OrgCache, OrgStore};
use betterlite_auth::auth_org_lookup::project::get_project_object;
use betterlite_auth::auth_prefetch::rows::CachedProject;
use betterlite_db::budget_permission_rows::LiteLLMBudgetTable;
use betterlite_db::database_availability::policy::AllowRequestsOnOutage;
use betterlite_db::resource_rows::organization::OrganizationTable;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use tracing::subscriber::set_default;
use tracing_subscriber::layer::SubscriberExt as _;

use self::fixtures::{Allow, Function, Input, Levels, Script, Step};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let script = Arc::new(Script::new(input));
        let store: Option<Arc<dyn OrgStore>> = if input.database {
            Some(Arc::clone(&script) as Arc<dyn OrgStore>)
        } else {
            None
        };
        let lookup = OrgLookup {
            store,
            cache: Arc::clone(&script) as Arc<dyn OrgCache>,
            in_memory_ttl: input.in_memory_ttl,
        };
        let levels = Levels {
            script: Arc::clone(&script),
        };
        let _logging = set_default(tracing_subscriber::registry().with(levels));
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .or_value_error()?;
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            script.begin(step);
            let (value, error) = runtime.block_on(outcome(input, &lookup, step));
            let board = script.board.lock().unwrap_or_else(PoisonError::into_inner);
            steps.push(json!({
                "value": value,
                "error": error,
                "events": board.events.clone(),
                "logs": board.logs.clone(),
            }));
        }
        Ok(json!({"steps": steps}))
    }
}

async fn outcome(input: &Input, lookup: &OrgLookup, step: &Step) -> (Value, Value) {
    let answer = match input.function {
        Function::OrgObject => get_org_object(
            lookup,
            OrgId::Text(input.target.clone()),
            input.include_budget,
        )
        .await
        .map(|org| org_view(org.as_ref())),
        Function::OrgAlias => get_org_object_by_alias(lookup, &input.target)
            .await
            .map(|org| org_view(Some(&org))),
        Function::OrgForRequest => get_org_object_for_request(lookup, &input.target, &policy(step))
            .await
            .map(|org| org_view(org.as_ref())),
        Function::ProjectObject => get_project_object(lookup, &input.target, step.now)
            .await
            .map(|project| project_view(project.as_ref())),
    };
    match answer {
        Ok(value) => (value, Value::Null),
        Err(error) => (Value::Null, failure(&error)),
    }
}

fn policy(step: &Step) -> AllowRequestsOnOutage {
    match &step.allow {
        Some(Allow::Boolean(value)) => AllowRequestsOnOutage::Boolean(*value),
        Some(Allow::Text(text)) => AllowRequestsOnOutage::Text(text.clone()),
        None => AllowRequestsOnOutage::Boolean(false),
    }
}

fn org_view(org: Option<&OrganizationTable>) -> Value {
    match org {
        None => Value::Null,
        Some(row) => json!({
            "organization_id": row.organization_id,
            "organization_alias": row.organization_alias,
            "models": row.models,
            "budget": budget_view(row.litellm_budget_table.as_ref()),
        }),
    }
}

fn budget_view(budget: Option<&LiteLLMBudgetTable>) -> Value {
    match budget {
        None => Value::Null,
        Some(row) => json!({
            "max_budget": row.max_budget,
            "allowed_models": row.allowed_models,
        }),
    }
}

fn project_view(project: Option<&CachedProject>) -> Value {
    match project {
        None => Value::Null,
        Some(row) => json!({
            "project_id": row.project.project_id,
            "project_alias": row.project.project_alias,
            "budget": budget_view(row.project.litellm_budget_table.as_ref()),
            "refreshed_at": row.last_refreshed_at,
        }),
    }
}

// the exception class, status and detail shape; the message text is not compared
fn failure(error: &OrgLookupError) -> Value {
    let kind = match error {
        OrgLookupError::NotFound { .. } => "org_not_found",
        OrgLookupError::AliasNotFound { .. }
        | OrgLookupError::AliasAmbiguous { .. }
        | OrgLookupError::AliasFailure { .. } => "http",
        OrgLookupError::NoDatabase | OrgLookupError::CacheRead(_) | OrgLookupError::Failure(_) => {
            "exception"
        }
    };
    match error.http() {
        Some(status) => json!({"kind": kind, "status": status, "detail": "object"}),
        None => json!({"kind": kind, "status": null, "detail": null}),
    }
}
