use betterlite_auth::user_roles::UserRole;
use betterlite_db::auxiliary_repository::repository::PrismaClient;
use betterlite_db::resource_repository::budget::BudgetRepository;

use crate::key_patch_fields::accessors::require_prisma_client;
use crate::team_access::caller::TeamCaller;

use super::error::KeyReassignmentError;

// ast-grep-ignore: no-long-names - _validate_end_user_budget_id_change keeps the source name: an unchanged or empty value is a no-op, and any other change is proxy-admin only
pub async fn validate_end_user_budget_id_change(
    requested_budget_id: Option<&str>,
    existing_budget_id: Option<&str>,
    caller: &TeamCaller,
    prisma_client: Option<PrismaClient<'_>>,
) -> Result<(), KeyReassignmentError> {
    let Some(requested) = requested_budget_id else {
        return Ok(());
    };
    if requested == existing_budget_id.unwrap_or("") {
        return Ok(());
    }
    if caller.user_role != Some(UserRole::ProxyAdmin) {
        return Err(KeyReassignmentError::EndUserBudgetForbidden);
    }
    if requested.is_empty() {
        return Ok(());
    }
    let found = BudgetRepository::new(Some(require_prisma_client(prisma_client)?))
        .find_by_id(requested)
        .await?;
    if found.is_none() {
        return Err(KeyReassignmentError::EndUserBudgetUnknown {
            budget_id: requested.to_owned(),
        });
    }
    Ok(())
}
