use super::tags::ChatConfigTag;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CohereRoute {
    V1,
    V2,
}

#[must_use]
pub fn get_cohere_route(model: &str) -> CohereRoute {
    if model.contains("v1/") {
        return CohereRoute::V1;
    }
    CohereRoute::V2
}

#[must_use]
pub fn select_config(model: &str) -> ChatConfigTag {
    match get_cohere_route(model) {
        CohereRoute::V1 => ChatConfigTag::CohereChat,
        CohereRoute::V2 => ChatConfigTag::CohereV2Chat,
    }
}
