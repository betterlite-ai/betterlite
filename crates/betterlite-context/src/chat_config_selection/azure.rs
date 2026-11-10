use super::facts::ModelFamilyProbe;
use super::tags::ChatConfigTag;

#[must_use]
pub fn is_o_series_model(model: &str) -> bool {
    if model.contains("o1") || model.contains("o3") || model.contains("o4") {
        return true;
    }
    model.contains("o_series/")
}

#[must_use]
pub fn select_config(detection_model: &str, families: &dyn ModelFamilyProbe) -> ChatConfigTag {
    if is_o_series_model(detection_model) {
        return ChatConfigTag::AzureOpenAiO1;
    }
    if families.azure_gpt_5(detection_model) {
        return ChatConfigTag::AzureOpenAiGpt5;
    }
    ChatConfigTag::AzureOpenAi
}
