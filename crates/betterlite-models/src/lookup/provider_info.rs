use crate::catalog::ModelCatalog;
use crate::fireworks_catalog_metadata::{Catalog, FireworksMetadata, MODEL_COST_KEY_PREFIX};

use super::capabilities::CapabilityView;

// The fireworks lookups read the fireworks_ai/ keys and the model's own key, so only those are copied.
fn fireworks_catalog(catalog: &ModelCatalog, model: &str) -> Catalog {
    catalog
        .lookup
        .iter()
        .filter(|(key, _)| key.starts_with(MODEL_COST_KEY_PREFIX) || key.as_str() == model)
        .filter_map(|(key, canonical)| {
            let entry = catalog.entries.get(canonical.as_ref())?;
            Some((key.clone(), entry.fields.clone()))
        })
        .collect()
}

// What get_provider_info answers for the providers that define it; the caller lays it over the catalog entry.
#[must_use]
pub fn provider_info(
    catalog: &ModelCatalog,
    model: &str,
    provider: Option<&str>,
) -> CapabilityView {
    match provider {
        Some("xai") => CapabilityView {
            supports_vision: None,
            supports_function_calling: None,
            supports_tool_choice: None,
            supports_response_schema: None,
            supports_prompt_caching: None,
            supports_reasoning: None,
            supports_web_search: Some(true),
            supports_pdf_input: None,
        },
        Some("fireworks_ai") => {
            let fireworks = fireworks_catalog(catalog, model);
            let info = FireworksMetadata::default().get_provider_info(&fireworks, 0, model);
            CapabilityView {
                supports_vision: info.supports_vision,
                supports_function_calling: Some(info.supports_function_calling),
                supports_tool_choice: None,
                supports_response_schema: None,
                supports_prompt_caching: Some(info.supports_prompt_caching),
                supports_reasoning: info.supports_reasoning,
                supports_web_search: None,
                supports_pdf_input: info.supports_pdf_input,
            }
        }
        _ => CapabilityView::default(),
    }
}
