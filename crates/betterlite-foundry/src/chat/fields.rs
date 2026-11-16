use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderField {
    pub field_name: &'static str,
    pub field_type: &'static str,
    pub field_description: &'static str,
    pub field_value: &'static str,
}

#[must_use]
pub fn required_params() -> Vec<ProviderField> {
    vec![
        ProviderField {
            field_name: "api_key",
            field_type: "string",
            field_description: "Your Azure AI Studio API Key.",
            field_value: "zEJ...",
        },
        ProviderField {
            field_name: "api_base",
            field_type: "string",
            field_description: "Your Azure AI Studio API Base.",
            field_value: "https://Mistral-serverless.",
        },
    ]
}
