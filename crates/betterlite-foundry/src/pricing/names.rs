const FEE_ENTRY_NAMES: [&str; 2] = ["model-router", "model_router"];
const FEE_LOOKUP_FALLBACK: &str = "model_router";
const PROVIDER_PREFIX: &str = "azure_ai/";

// Any position counts here, which is why a deployment named after the router is not a fee entry.
#[must_use]
pub fn is_azure_model_router(model: &str) -> bool {
    let lowered = model.to_lowercase();
    let hyphenated = lowered.contains("model-router");
    let underscored = lowered.contains("model_router");
    hyphenated || underscored
}

// removeprefix drops exactly one prefix, so a doubled azure_ai/ leaves a name that is no entry.
#[must_use]
pub fn is_router_fee_entry(model: &str) -> bool {
    let lowered = model.to_lowercase();
    let entry = lowered.strip_prefix(PROVIDER_PREFIX).unwrap_or(&lowered);
    FEE_ENTRY_NAMES.contains(&entry)
}

#[must_use]
pub fn router_fee_entry_name(model: &str) -> &'static str {
    let lowered = model.to_lowercase();
    let entry = lowered.strip_prefix(PROVIDER_PREFIX).unwrap_or(&lowered);
    if entry == "model-router" {
        return "model-router";
    }
    FEE_LOOKUP_FALLBACK
}

// The response name is the model whose tokens were priced, so it outranks the requested router.
#[must_use]
pub fn router_fee_name<'request>(
    model: &'request str,
    request_model: Option<&'request str>,
) -> Option<&'request str> {
    if is_router_fee_entry(model) {
        return None;
    }
    if is_azure_model_router(model) {
        return Some(model);
    }
    request_model.filter(|candidate| is_azure_model_router(candidate))
}
