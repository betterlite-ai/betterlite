mod tests;

use std::sync::PoisonError;

use betterlite_policy::pipeline::GuardrailPipeline;
use betterlite_policy::records::crud::{PolicyConditionRequest, PolicyCreateRequest};
use betterlite_policy::records::db::PolicyDBResponse;
use betterlite_policy::registry::PolicySource;
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyRow, PolicyTable as _, PolicyTableSource, policy_db_response,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_with::skip_serializing_none;

use super::error::Operation;
use super::request_pipeline::validate_pipeline;
use super::stored_policies::parse_stored_policy;
use super::{PolicyError, PolicyStore};

#[skip_serializing_none]
#[derive(Serialize)]
struct NewPolicy<'request> {
    policy_name: &'request str,
    version_number: i32,
    version_status: &'static str,
    is_latest: bool,
    production_at: DateTime<Utc>,
    guardrails_add: &'request [String],
    guardrails_remove: &'request [String],
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    inherit: Option<&'request str>,
    description: Option<&'request str>,
    created_by: Option<&'request str>,
    updated_by: Option<&'request str>,
    condition: Option<&'request PolicyConditionRequest>,
    pipeline: Option<&'request GuardrailPipeline>,
}

pub async fn add_policy_to_db<Client>(
    store: &PolicyStore<'_, Client>,
    request: &PolicyCreateRequest,
    created_by: Option<&str>,
) -> Result<PolicyDBResponse, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let created = insert_policy(store, request, created_by).await?;
    let response = policy_db_response(&created);
    // the stored row holds the request's fields after validation, so it parses to the same policy
    let policy = parse_stored_policy::<Client>(&response, Operation::AddPolicy)?;
    store
        .registry
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .add_policy(&response.policy_name, policy, PolicySource::Db);
    Ok(response)
}

async fn insert_policy<Client>(
    store: &PolicyStore<'_, Client>,
    request: &PolicyCreateRequest,
    created_by: Option<&str>,
) -> Result<PolicyRow, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let chain = || vec![Operation::AddPolicy];
    let pipeline = validate_pipeline(request.pipeline.as_ref())
        .map_err(|source| PolicyError::<Client>::Json(chain(), source).logged())?;
    let now = (store.now)();
    let row = NewPolicy {
        policy_name: &request.policy_name,
        version_number: 1,
        version_status: "production",
        is_latest: true,
        production_at: now,
        guardrails_add: request.guardrails_add.as_deref().unwrap_or_default(),
        guardrails_remove: request.guardrails_remove.as_deref().unwrap_or_default(),
        created_at: now,
        updated_at: now,
        inherit: request.inherit.as_deref(),
        description: request.description.as_deref(),
        created_by,
        updated_by: created_by,
        condition: request.condition.as_ref(),
        pipeline: pipeline.as_ref(),
    };
    let data = serde_json::to_string(&row)
        .and_then(|text| serde_json::from_str::<PolicyClause>(&text))
        .map_err(|source| PolicyError::<Client>::Json(chain(), source).logged())?;
    let table = store.client.policy_table();
    table
        .create(data)
        .await
        .map_err(|source| PolicyError::<Client>::Database(chain(), source).logged())
}
