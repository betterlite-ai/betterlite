use std::collections::BTreeSet;

// SpecialModelNames: the three sentinels a key or team grant may carry instead of a model name.
pub const ALL_TEAM_MODELS: &str = "all-team-models";
pub const ALL_PROXY_MODELS: &str = "all-proxy-models";
pub const NO_DEFAULT_MODELS: &str = "no-default-models";

// `list(dict.fromkeys(...))`: the first occurrence keeps its place, later ones disappear.
pub fn dedup_first(models: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.clone()))
        .collect()
}

// append_unique: no-default-models is never listed, and every other name keeps its first place.
pub fn append_unique(models: &mut Vec<String>, additions: impl IntoIterator<Item = String>) {
    for model in additions {
        if model != NO_DEFAULT_MODELS && !models.contains(&model) {
            models.push(model);
        }
    }
}
