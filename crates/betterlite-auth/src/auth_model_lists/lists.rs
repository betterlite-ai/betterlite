use std::collections::BTreeSet;
// SpecialModelNames.no_default_models; the two grant sentinels already live in auth_model_patterns.
pub const NO_DEFAULT_MODELS: &str = "no-default-models";
// `list(dict.fromkeys(...))`: the first occurrence keeps its place, later ones disappear.
#[must_use]
pub fn dedup_first(models: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    models
        .into_iter()
        .filter(|model| seen.insert(model.clone()))
        .collect()
}
pub fn append_unique<Additions>(models: &mut Vec<String>, additions: Additions)
where
    Additions: IntoIterator<Item = String>,
{
    for model in additions {
        if model != NO_DEFAULT_MODELS && !models.contains(&model) {
            models.push(model);
        }
    }
}
