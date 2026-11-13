#![cfg(test)]

use crate::error_responses::{
    ErrorStatus, RATE_LIMIT_DESCRIPTION, error_response_schema, error_responses,
};
use crate::schema_transforms::tree::JsonObject;

#[test]
fn the_table_keeps_litellm_status_order_with_the_last_class_winning_each_status() {
    let table = error_responses();
    let statuses = table.iter().map(|entry| entry.status).collect::<Vec<u16>>();
    assert_eq!(statuses, vec![401, 404, 400, 422, 408, 403, 429, 500, 503]);
    let descriptions = table
        .iter()
        .map(|entry| entry.description.as_str())
        .collect::<Vec<&str>>();
    assert_eq!(
        descriptions,
        vec![
            "AuthenticationError",
            "NotFoundError",
            "ContentPolicyViolationError",
            "UnprocessableEntityError",
            "Timeout",
            "PermissionDeniedError",
            RATE_LIMIT_DESCRIPTION,
            "JSONSchemaValidationError",
            "APIConnectionError",
        ]
    );
}

#[test]
fn the_rate_limit_description_is_the_cleaned_docstring() {
    assert_eq!(
        RATE_LIMIT_DESCRIPTION,
        "Unified rate-limit error.\n\n\
         Every rate-limit condition surfaced by litellm \u{2014} whether it originated from\n\
         an upstream LLM provider, a vendor batch endpoint, or one of litellm's own\n\
         proxy-side limiters (parallel-requests, dynamic-rate, batch-rate, budget,\n\
         max-iterations, etc.) \u{2014} is raised as an instance of this class.\n\n\
         The :attr:`category` attribute lets callers distinguish the source. See\n\
         :class:`RateLimitErrorCategory` for the available values."
    );
}

#[test]
fn the_error_response_schema_is_the_component_fastapi_emits() {
    let expected: JsonObject = serde_json::from_str(
        r#"{
            "properties": {
                "detail": {
                    "additionalProperties": true,
                    "type": "object",
                    "title": "Detail",
                    "example": {
                        "error": {
                            "code": "error_code",
                            "message": "Error message",
                            "param": "error_param",
                            "type": "error_type"
                        }
                    }
                }
            },
            "type": "object",
            "required": ["detail"],
            "title": "ErrorResponse"
        }"#,
    )
    .expect("the expected schema is valid JSON");
    assert_eq!(error_response_schema(), expected);
}

#[test]
fn a_status_entry_serializes_as_status_and_description() {
    let entry = ErrorStatus {
        status: 408,
        description: "Timeout".to_owned(),
    };
    let serialized = serde_json::to_string(&entry).expect("an error status serializes");
    assert_eq!(serialized, r#"{"status":408,"description":"Timeout"}"#);
}
