#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Titan,
    Stability1,
    Stability3,
    Nova,
}

// A None/empty model is Python's falsy-model case, which always resolves to Stability1
#[must_use]
pub fn select_family(model: Option<&str>) -> Family {
    let Some(name) = model.filter(|text| !text.is_empty()) else {
        return Family::Stability1;
    };
    if name.contains("amazon.titan") {
        Family::Titan
    } else if name.contains("amazon.nova-canvas") {
        Family::Nova
    } else if name.contains("sd3") || name.contains("sd3.5") || name.contains("stable-image") {
        Family::Stability3
    } else {
        Family::Stability1
    }
}
