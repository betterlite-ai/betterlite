use indexmap::IndexMap;
use rust_decimal::Decimal;

use crate::adaptive_classification::config::AdaptiveRouterPreferences;
use crate::adaptive_selection::default_preferences;

// one Router model_list row, as _ensure_adaptive_router reads it
#[derive(Debug, Clone, PartialEq)]
pub struct ModelDeclaration {
    pub model_name: String,
    pub preferences: Option<AdaptiveRouterPreferences>,
    pub params_cost: Option<Decimal>,
    pub info_cost: Option<Decimal>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelFacts {
    pub preferences: IndexMap<String, AdaptiveRouterPreferences>,
    pub costs: IndexMap<String, Decimal>,
}

impl ModelFacts {
    // L2960-2985: the first deployment of a group declares its preferences and price, and
    // litellm_params.input_cost_per_token wins over model_info's copy when both are set.
    #[must_use]
    pub fn from_declarations(declarations: &[ModelDeclaration]) -> Self {
        let mut facts = Self::default();
        for declaration in declarations {
            if facts.preferences.contains_key(&declaration.model_name) {
                continue;
            }
            facts.preferences.insert(
                declaration.model_name.clone(),
                declaration
                    .preferences
                    .clone()
                    .unwrap_or_else(default_preferences),
            );
            facts.costs.insert(
                declaration.model_name.clone(),
                declaration
                    .params_cost
                    .or(declaration.info_cost)
                    .unwrap_or(Decimal::ZERO),
            );
        }
        facts
    }
}
