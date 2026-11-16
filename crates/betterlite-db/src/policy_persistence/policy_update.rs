mod tests;

use betterlite_policy::pipeline::GuardrailPipeline;
use betterlite_policy::records::crud::{PolicyConditionRequest, PolicyUpdateRequest};
use betterlite_policy::records::db::PolicyDBResponse;
use betterlite_policy::row_adapter::{
    PolicyClause, PolicyTable as _, PolicyTableSource, policy_db_response,
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_with::skip_serializing_none;

use super::error::Operation;
use super::existing_policy::existing_policy;
use super::request_pipeline::validate_pipeline;
use super::{PolicyError, PolicyStore};

#[skip_serializing_none]
#[derive(Serialize)]
struct PolicyChanges<'request> {
    updated_at: DateTime<Utc>,
    // LiteLLM always writes updated_by, null included
    #[serialize_always]
    updated_by: Option<&'request str>,
    policy_name: Option<&'request str>,
    inherit: Option<&'request str>,
    description: Option<&'request str>,
    guardrails_add: Option<&'request [String]>,
    guardrails_remove: Option<&'request [String]>,
    condition: Option<&'request PolicyConditionRequest>,
    pipeline: Option<GuardrailPipeline>,
}

pub async fn update_policy_in_db<Client>(
    store: &PolicyStore<'_, Client>,
    policy_id: &str,
    request: &PolicyUpdateRequest,
    updated_by: Option<&str>,
) -> Result<PolicyDBResponse, PolicyError<Client>>
where
    Client: PolicyTableSource,
{
    let chain = || vec![Operation::UpdatePolicy];
    let (filter, existing) = existing_policy(store, policy_id, Operation::UpdatePolicy).await?;
    let status = existing
        .version_status
        .unwrap_or_else(|| "production".to_owned());
    if status != "draft" {
        return Err(PolicyError::<Client>::NotDraft(chain(), status).logged());
    }
    let changes = PolicyChanges {
        updated_at: (store.now)(),
        updated_by,
        policy_name: request.policy_name.as_deref(),
        inherit: request.inherit.as_deref(),
        description: request.description.as_deref(),
        guardrails_add: request.guardrails_add.as_deref(),
        guardrails_remove: request.guardrails_remove.as_deref(),
        condition: request.condition.as_ref(),
        pipeline: validate_pipeline(request.pipeline.as_ref())
            .map_err(|source| PolicyError::<Client>::Json(chain(), source).logged())?,
    };
    let data = serde_json::to_string(&changes)
        .and_then(|text| serde_json::from_str::<PolicyClause>(&text))
        .map_err(|source| PolicyError::<Client>::Json(chain(), source).logged())?;
    let table = store.client.policy_table();
    let updated = table
        .update(filter, data)
        .await
        .map_err(|source| PolicyError::<Client>::Database(chain(), source).logged())?;
    Ok(policy_db_response(&updated))
}
