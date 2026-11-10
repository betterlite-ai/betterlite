mod tests;

pub trait BedrockCatalog {
    fn base_model(&self, model: &str) -> String;
    fn prices_as_bedrock(&self, base_model: &str) -> bool;
}

#[must_use]
pub fn budget_model_candidates(model: &str, catalog: &dyn BedrockCatalog) -> Vec<String> {
    let without_provider = model.rsplit('/').next().unwrap_or(model);
    let named = [model.to_owned(), without_provider.to_owned()];
    let mut candidates: Vec<String> = Vec::new();
    for candidate in named.into_iter().chain(bedrock_candidates(model, catalog)) {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn bedrock_candidates(model: &str, catalog: &dyn BedrockCatalog) -> Vec<String> {
    let base_model = catalog.base_model(model);
    if !catalog.prices_as_bedrock(&base_model) {
        return Vec::new();
    }
    match base_model.split_once('.') {
        Some((_, without_vendor)) if !without_vendor.is_empty() => {
            vec![base_model.clone(), without_vendor.to_owned()]
        }
        _ => vec![base_model],
    }
}
