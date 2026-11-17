use std::collections::{BTreeMap, BTreeSet};

use crate::complexity_math;

// _RequestContextFit: the per-group context windows and the token count the placement may not exceed
#[derive(Debug, Clone, PartialEq)]
pub struct RequestContextFit {
    pub windows: BTreeMap<String, Option<u64>>,
    pub needed: Option<u64>,
    pub buffer: f64,
}

impl RequestContextFit {
    // accepts: an unknown window or token count always fits; a known window must hold the buffered request
    #[must_use]
    pub fn accepts(&self, model: &str) -> bool {
        let window = self.windows.get(model).copied().flatten();
        complexity_math::window_holds(window, self.needed, self.buffer)
    }
}

// _allowed: the pool keeps its order, and without a fit filter every model is allowed
#[must_use]
pub fn allowed(models: &[String], fit_filter: Option<&BTreeSet<String>>) -> Vec<String> {
    let Some(fit_filter) = fit_filter else {
        return models.to_vec();
    };
    models
        .iter()
        .filter(|model| fit_filter.contains(model.as_str()))
        .cloned()
        .collect()
}
