use betterlite_auth::auth_model_patterns::ALL_PROXY_MODELS;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::budget_permission_rows::{JsonData, LiteLLMBudgetTable};
use betterlite_db::resource_rows::organization::OrganizationTable;

use super::allocation::{check_model_allocations, check_rate_allocations};
use super::error::{Rejection, TeamLimitError};
use super::request::TeamLimitRequest;
use crate::team_fields::tables::team_table;
use crate::team_request_contracts::values::ThroughputLimitType;

const GUARANTEED_THROUGHPUT: &str = "guaranteed_throughput";

// the organization's teams are read only for a guaranteed-throughput request, and the read includes the team being updated
pub async fn check_org_team_limits(
    organization: &OrganizationTable,
    request: &TeamLimitRequest<'_>,
    client: Option<PrismaClient<'_>>,
) -> Result<(), TeamLimitError> {
    check_direct_limits(organization, request)?;
    let rpm_guaranteed = is_guaranteed(request.rpm_limit_type(), request, "rpm_limit_type");
    let tpm_guaranteed = is_guaranteed(request.tpm_limit_type(), request, "tpm_limit_type");
    if !rpm_guaranteed && !tpm_guaranteed {
        return Ok(());
    }
    let organization_id = organization
        .organization_id
        .as_deref()
        .ok_or(TeamLimitError::MissingOrganization)?;
    let teams = team_table(client)?
        .find_by_organization_id(organization_id)
        .await?;
    check_model_allocations(&teams, organization, request)?;
    check_rate_allocations(&teams, organization, request)?;
    Ok(())
}

// budget, then models, then TPM, then RPM, each against the organization's own ceiling
fn check_direct_limits(
    organization: &OrganizationTable,
    request: &TeamLimitRequest<'_>,
) -> Result<(), Rejection> {
    let budget = organization.litellm_budget_table.as_ref();
    let name = organization.organization_id.clone().unwrap_or_default();
    let budget_ceiling = budget.and_then(|table| table.max_budget);
    if let (Some(requested), Some(ceiling)) = (request.max_budget(), budget_ceiling)
        && requested > ceiling
    {
        return Err(Rejection::OrganizationBudget {
            requested,
            ceiling,
            organization: name,
        });
    }
    let models = request.models();
    if let Some(model) = unlisted_model(models.as_deref(), &organization.models) {
        return Err(Rejection::OrganizationModel {
            model: model.clone(),
            allowed: organization.models.clone(),
            organization: name,
        });
    }
    check_direct_rates(budget, request, name)
}

fn check_direct_rates(
    budget: Option<&LiteLLMBudgetTable>,
    request: &TeamLimitRequest<'_>,
    organization: String,
) -> Result<(), Rejection> {
    let tpm_ceiling = budget.and_then(|table| table.tpm_limit);
    if let (Some(requested), Some(ceiling)) = (request.tpm_limit(), tpm_ceiling)
        && requested > ceiling
    {
        return Err(Rejection::OrganizationTpm {
            requested,
            ceiling,
            organization,
        });
    }
    let rpm_ceiling = budget.and_then(|table| table.rpm_limit);
    if let (Some(requested), Some(ceiling)) = (request.rpm_limit(), rpm_ceiling)
        && requested > ceiling
    {
        return Err(Rejection::OrganizationRpm {
            requested,
            ceiling,
            organization,
        });
    }
    Ok(())
}

// an empty organization list or one holding all-proxy-models allows every model
fn unlisted_model<'model>(
    requested: Option<&'model [JsonData]>,
    allowed: &[String],
) -> Option<&'model JsonData> {
    if allowed.is_empty() || allowed.iter().any(|model| model == ALL_PROXY_MODELS) {
        return None;
    }
    requested?
        .iter()
        .find(|model| !matches!(model, JsonData::String(name) if allowed.contains(name)))
}

// a request field that is set wins over the metadata key, as LiteLLM's `field or metadata.get(key)` does
fn is_guaranteed(
    field: Option<ThroughputLimitType>,
    request: &TeamLimitRequest<'_>,
    key: &str,
) -> bool {
    match field {
        Some(kind) => kind == ThroughputLimitType::Guaranteed,
        None => matches!(
            request.metadata().and_then(|entries| entries.get(key)),
            Some(JsonData::String(text)) if text == GUARANTEED_THROUGHPUT
        ),
    }
}
