use betterlite_config::sections::classifier_message_text as section;
use betterlite_router::classifier_message_text::markers::MarkerPair;

mod tests;

#[must_use]
pub fn deployment_markers(
    settings: &section::Settings,
    deployment: usize,
) -> Option<Vec<MarkerPair<'_>>> {
    settings
        .deployments
        .get(deployment)
        .and_then(Option::as_ref)
        .map(markers)
}

fn markers(configured: &[section::MarkerPair]) -> Vec<MarkerPair<'_>> {
    configured
        .iter()
        .map(|pair| MarkerPair {
            open: pair.open.as_str(),
            close: pair.close.as_str(),
        })
        .collect()
}
