use std::collections::BTreeSet;

use super::reconcile::GuardrailReconcile;
use super::runtime::GuardrailRuntime;
use crate::guardrail_registry::rows::GuardrailRows;

// a free function, since the workspace denies a second inherent impl of GuardrailRuntime
pub async fn poll_database<R: GuardrailRows>(runtime: &GuardrailRuntime<R>) {
    let _serialised = runtime.reconcile_lock.lock().await;
    let guardrails = match runtime.registry.get_all_guardrails_from_db().await {
        Ok(guardrails) => guardrails,
        Err(error) => {
            // reconciling after a failed read would purge every db guardrail from memory
            tracing::error!("guardrail poll could not read the database: {error}");
            return;
        }
    };
    let mut db_guardrail_ids = BTreeSet::new();
    for guardrail in guardrails {
        let guardrail_id = guardrail.guardrail_id.clone().filter(|id| !id.is_empty());
        if let Some(id) = &guardrail_id {
            db_guardrail_ids.insert(id.clone());
        }
        let guardrail_name = guardrail.guardrail_name.clone();
        let synced = runtime
            .handler
            .write()
            .await
            .sync_guardrail_from_db(guardrail);
        if let Err(error) = synced {
            tracing::error!(
                "skipping guardrail '{guardrail_name}' (ID: {}): {error}",
                guardrail_id.as_deref().unwrap_or("None")
            );
        }
    }
    runtime
        .handler
        .write()
        .await
        .reconcile_db_guardrails(&db_guardrail_ids);
}
