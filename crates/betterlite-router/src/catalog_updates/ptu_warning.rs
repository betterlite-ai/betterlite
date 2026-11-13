use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_models::ptu_pricing::{PTU_COST_ATTRIBUTION_ENV_VAR, PtuModelInfo, declares_ptu};
use betterlite_models::tiered_pricing::Scalar;

use super::entries::ModelEntry;

const PTU_FIELDS: [&str; 4] = [
    "ptu_count",
    "cost_per_ptu_per_hour",
    "ptu_effective_from",
    "ptu_effective_to",
];

// The warning at the top of set_model_list: config.yaml entries with PTU terms are billed per token while attribution is off.
pub fn warn_unattributed_ptu(entries: &[ModelEntry], ptu_cost_attribution: bool) {
    if ptu_cost_attribution {
        return;
    }
    let mut names = Vec::new();
    for entry in entries {
        // only a null counts as undeclared, so the value itself does not matter
        let declared: BTreeMap<String, Scalar> = PTU_FIELDS
            .iter()
            .filter(|field| {
                entry
                    .model_info
                    .get(**field)
                    .is_some_and(|value| !matches!(value, Json::Null))
            })
            .map(|field| ((*field).to_owned(), Scalar::Bool(true)))
            .collect();
        let db_model = entry.model_info.get("db_model") == Some(&Json::Bool(true));
        if !db_model && declares_ptu(&PtuModelInfo::from(&declared)) {
            names.push(entry.model_name.as_str());
        }
    }
    if names.is_empty() {
        return;
    }
    tracing::warn!(
        "PTU fields are set on config.yaml deployment(s) {}, but PTU cost attribution is disabled, so no flat cost accrues and this traffic is billed per token. Set {PTU_COST_ATTRIBUTION_ENV_VAR}=True to enable it",
        names.join(", "),
    );
}
