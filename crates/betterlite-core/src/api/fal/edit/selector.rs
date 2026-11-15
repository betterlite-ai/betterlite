#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Config {
    Generic,
    FluxLoraDepth,
}

pub const DEPTH_MARKER: &str = "flux-lora-depth";

#[must_use]
pub fn for_model(model: &str) -> Config {
    if model.to_lowercase().contains(DEPTH_MARKER) {
        Config::FluxLoraDepth
    } else {
        Config::Generic
    }
}
