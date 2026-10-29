#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Titan,
    Stability1,
    Stability3,
    Nova,
}

#[must_use]
pub fn is_titan_model(model: &str) -> bool {
    model.contains("amazon.titan")
}

#[must_use]
pub fn is_nova_model(model: &str) -> bool {
    model.contains("amazon.nova-canvas")
}

#[must_use]
pub fn is_stability3_model(model: &str) -> bool {
    model.contains("sd3") || model.contains("sd3.5") || model.contains("stable-image")
}

/// `model` is `None`/empty for Python's falsy-model case, which always resolves to Stability1.
#[must_use]
pub fn select_family(model: Option<&str>) -> Family {
    let Some(name) = model.filter(|text| !text.is_empty()) else {
        return Family::Stability1;
    };
    if is_titan_model(name) {
        Family::Titan
    } else if is_nova_model(name) {
        Family::Nova
    } else if is_stability3_model(name) {
        Family::Stability3
    } else {
        Family::Stability1
    }
}
