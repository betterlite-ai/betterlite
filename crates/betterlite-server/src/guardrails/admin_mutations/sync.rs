use betterlite_config::sections::guardrail_params::records::Guardrail;
use betterlite_guardrails::guardrail_registry::error::RegistryError;
use betterlite_guardrails::guardrail_registry::rows::GuardrailRows;
use betterlite_guardrails::guardrail_startup::reconcile::GuardrailReconcile as _;

use super::MutationDatabase;
use super::error::is_configuration_error;
use super::review_rows::ReviewRows;

// Only a configuration error comes back, since LiteLLM answers every other sync failure with a warning
pub(super) async fn sync_in_memory<R, S>(
    database: &MutationDatabase<R, S>,
    guardrail_id: &str,
    guardrail: Guardrail,
) -> Result<(), RegistryError>
where
    R: GuardrailRows,
    S: ReviewRows,
{
    let name = guardrail.guardrail_name.clone();
    let synced = database
        .runtime
        .handler()
        .write()
        .await
        .sync_guardrail_from_db(guardrail);
    match synced {
        Ok(_held) => tracing::info!(
            "Immediate sync: Successfully updated guardrail '{name}' (ID: {guardrail_id})"
        ),
        Err(error) => {
            if is_configuration_error(&error) {
                return Err(error);
            }
            tracing::warn!(
                "Immediate sync: Failed to update '{name}' (ID: {guardrail_id}) in memory: {error}"
            );
        }
    }
    Ok(())
}
