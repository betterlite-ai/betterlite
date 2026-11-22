const EDIT_MARKERS: [&str; 11] = [
    "upscale",
    "outpaint",
    "inpaint",
    "erase",
    "remove-background",
    "search-recolor",
    "search-replace",
    "control-sketch",
    "control-structure",
    "style-guide",
    "style-transfer",
];

// The model must name the provider and one edit family; there is no catalog lookup.
#[must_use]
pub fn is_stability_edit_model(model: Option<&str>) -> bool {
    let Some(name) = model else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    let lower = name.to_lowercase();
    lower.contains("stability.") && EDIT_MARKERS.iter().any(|marker| lower.contains(marker))
}
