use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_guardrails::guardrail_startup::router_list::RouterGuardrail;
use betterlite_guardrails::router_guardrail_selection::select_router_guardrail;
use rand::Rng;

use super::execute::{BalanceError, GuardrailBalancer};
use crate::guardrails::callback_caps::traits::ProxyCallback;

// the router's guardrail_list; LiteLLM finds each deployment's callback by its guardrail_id
pub struct RouterGuardrailBalancer<Random> {
    pub guardrails: Vec<RouterGuardrail>,
    pub callbacks_by_id: BTreeMap<String, Arc<dyn ProxyCallback>>,
    pub rng: Mutex<Random>,
}

impl<R: Rng + Send> GuardrailBalancer for RouterGuardrailBalancer<Random> {
    fn should_use_load_balancing(&self, guardrail_name: &str) -> bool {
        let deployments = self
            .guardrails
            .iter()
            .filter(|guardrail| guardrail.guardrail_name == guardrail_name);
        deployments.count() > 1
    }

    fn available_guardrail(
        &self,
        guardrail_name: &str,
    ) -> Result<Arc<dyn ProxyCallback>, BalanceError> {
        let selected = select_router_guardrail(
            &self.guardrails,
            guardrail_name,
            &mut *self.rng.lock().unwrap_or_else(PoisonError::into_inner),
        )?;
        selected
            .id
            .as_ref()
            .and_then(|id| self.callbacks_by_id.get(id))
            .map(Arc::clone)
            .ok_or_else(|| BalanceError::NoCallback {
                guardrail_name: guardrail_name.to_owned(),
            })
    }
}
