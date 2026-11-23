use std::sync::{Mutex, PoisonError};

use async_trait::async_trait;
use betterlite_server::key_patch_fields::error::KeyPatchError;
use betterlite_server::key_patch_fields::soft_budget::SoftBudgetStore;
use rust_decimal::Decimal;
use serde_json::{Value, json};

use super::support::amount_value;

#[derive(Default)]
pub(super) struct RecordingStore {
    effects: Mutex<Vec<Value>>,
}

impl RecordingStore {
    fn record(&self, effect: Value) {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(effect);
    }

    pub(super) fn snapshot(&self) -> Vec<Value> {
        self.effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl SoftBudgetStore for RecordingStore {
    async fn update_soft_budget(
        &self,
        budget_id: &str,
        soft_budget: Option<Decimal>,
        updated_by: &str,
    ) -> Result<(), KeyPatchError> {
        self.record(json!({
            "kind": "update",
            "where": {"budget_id": budget_id},
            "data": {"soft_budget": amount_value(soft_budget), "updated_by": updated_by},
        }));
        Ok(())
    }

    async fn create_soft_budget(
        &self,
        soft_budget: Decimal,
        created_by: &str,
    ) -> Result<String, KeyPatchError> {
        let budget_id = String::from("budget-created");
        self.record(json!({
            "kind": "create",
            "data": {
                "soft_budget": amount_value(Some(soft_budget)),
                "created_by": created_by,
                "updated_by": created_by,
            },
        }));
        Ok(budget_id)
    }
}
