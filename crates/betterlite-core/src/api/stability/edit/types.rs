use betterlite_models::decisions::Json;
use betterlite_openai::api::image_contracts::records::ImageFile;
use indexmap::IndexMap;

pub const DEFAULT_API_BASE: &str = "https://api.stability.ai";
pub const DEFAULT_EDIT_ENDPOINT: &str = "/v2beta/stable-image/edit/inpaint";

// LiteLLM's order matters: style precedes style-transfer, so a transfer model reaches /control/style
pub const STABILITY_EDIT_ENDPOINTS: [(&str, &str); 14] = [
    ("inpaint", "/v2beta/stable-image/edit/inpaint"),
    ("outpaint", "/v2beta/stable-image/edit/outpaint"),
    ("erase", "/v2beta/stable-image/edit/erase"),
    (
        "search-and-replace",
        "/v2beta/stable-image/edit/search-and-replace",
    ),
    (
        "search-and-recolor",
        "/v2beta/stable-image/edit/search-and-recolor",
    ),
    (
        "remove-background",
        "/v2beta/stable-image/edit/remove-background",
    ),
    (
        "replace-background-and-relight",
        "/v2beta/stable-image/edit/replace-background-and-relight",
    ),
    ("fast", "/v2beta/stable-image/upscale/fast"),
    ("conservative", "/v2beta/stable-image/upscale/conservative"),
    ("creative", "/v2beta/stable-image/upscale/creative"),
    ("sketch", "/v2beta/stable-image/control/sketch"),
    ("structure", "/v2beta/stable-image/control/structure"),
    ("style", "/v2beta/stable-image/control/style"),
    (
        "style-transfer",
        "/v2beta/stable-image/control/style-transfer",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditValue {
    Json(Json),
    Files(Vec<ImageFile>),
}

pub type EditOptions = IndexMap<String, EditValue>;

#[must_use]
pub fn model_endpoint(model: &str) -> &'static str {
    let lowered = model.to_lowercase();
    let name = lowered.strip_prefix("stability/").unwrap_or(&lowered);
    STABILITY_EDIT_ENDPOINTS
        .iter()
        .find_map(|(key, endpoint)| name.contains(key).then_some(*endpoint))
        .unwrap_or(DEFAULT_EDIT_ENDPOINT)
}
