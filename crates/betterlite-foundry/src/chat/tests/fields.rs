use crate::chat::fields::required_params;

use super::support::body;

pub(super) fn foundry_required_fields() {
    let fields = required_params();
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].field_name, "api_key");
    assert_eq!(fields[0].field_type, "string");
    assert_eq!(fields[0].field_description, "Your Azure AI Studio API Key.");
    assert_eq!(fields[0].field_value, "zEJ...");
    assert_eq!(fields[1].field_name, "api_base");
    assert_eq!(fields[1].field_type, "string");
    assert_eq!(
        fields[1].field_description,
        "Your Azure AI Studio API Base."
    );
    assert_eq!(fields[1].field_value, "https://Mistral-serverless.");
    let encoded = serde_json::to_value(fields[0]).expect("the field encodes");
    assert_eq!(
        serde_json::from_value(encoded).expect("the field reads as JSON"),
        body(
            r#"{"field_name":"api_key","field_type":"string",
                "field_description":"Your Azure AI Studio API Key.","field_value":"zEJ..."}"#
        )
    );
}
