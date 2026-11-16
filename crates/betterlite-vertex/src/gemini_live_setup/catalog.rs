use betterlite_models::catalog::{CatalogEntry, ModelCatalog};
use betterlite_models::metadata::identity::Mode;

#[must_use]
pub fn is_audio_only(catalog: &ModelCatalog, model: &str) -> bool {
    cost_entry(catalog, model).is_some_and(|entry| {
        let media = &entry.fields.capabilities.media;
        media.gemini_native_audio == Some(true) || media.gemini_audio_only_live == Some(true)
    })
}

#[must_use]
pub fn is_text_only(catalog: &ModelCatalog, model: &str) -> bool {
    cost_entry(catalog, model)
        .is_some_and(|entry| entry.fields.metadata.mode == Some(Mode::AudioTranscription))
}

#[must_use]
pub fn default_response_modality(catalog: &ModelCatalog, model: &str) -> &'static str {
    if is_text_only(catalog, model) {
        "TEXT"
    } else {
        "AUDIO"
    }
}

#[must_use]
pub fn coerce_response_modalities(
    catalog: &ModelCatalog,
    model: &str,
    modalities: &[String],
) -> Vec<String> {
    let normalized: Vec<String> = modalities
        .iter()
        .map(|modality| modality.to_uppercase())
        .collect();
    if is_audio_only(catalog, model) && normalized.iter().any(|modality| modality == "TEXT") {
        return without(&normalized, "TEXT", "AUDIO");
    }
    if is_text_only(catalog, model) && normalized.iter().any(|modality| modality == "AUDIO") {
        return without(&normalized, "AUDIO", "TEXT");
    }
    normalized
}

fn without(modalities: &[String], removed: &str, fallback: &str) -> Vec<String> {
    let kept: Vec<String> = modalities
        .iter()
        .filter(|modality| modality.as_str() != removed)
        .cloned()
        .collect();
    if kept.is_empty() {
        vec![fallback.to_owned()]
    } else {
        kept
    }
}

fn cost_entry<'catalog>(
    catalog: &'catalog ModelCatalog,
    model: &str,
) -> Option<&'catalog CatalogEntry> {
    if let Some(entry) = catalog.entries.get(model) {
        return Some(entry);
    }
    let stripped = model.split_once('/').map_or(model, |(_prefix, rest)| rest);
    catalog
        .entries
        .get(stripped)
        .or_else(|| catalog.entries.get(&format!("gemini/{stripped}")))
}
