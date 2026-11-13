use serde::Deserialize;

const CHAT_MODE: &str = "chat";
const EMBEDDING_MODE: &str = "embedding";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredModel {
    pub name: String,
    pub mode: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct RawModelListing {
    pub id: String,
    #[serde(default = "default_mode")]
    pub mode: String,
}

fn default_mode() -> String {
    CHAT_MODE.to_owned()
}

#[must_use]
pub fn parse_discovered_models(raw: &[RawModelListing]) -> Vec<DiscoveredModel> {
    raw.iter()
        .map(|listing| DiscoveredModel {
            name: listing.id.clone(),
            mode: listing.mode.clone(),
        })
        .collect()
}

#[must_use]
pub fn chat_models(models: &[DiscoveredModel]) -> Vec<DiscoveredModel> {
    models
        .iter()
        .filter(|model| model.mode == CHAT_MODE)
        .cloned()
        .collect()
}

#[must_use]
pub fn embedding_models(models: &[DiscoveredModel]) -> Vec<DiscoveredModel> {
    models
        .iter()
        .filter(|model| model.mode == EMBEDDING_MODE)
        .cloned()
        .collect()
}
