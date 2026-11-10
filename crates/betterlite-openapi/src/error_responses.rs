use serde::{Deserialize, Serialize};

use crate::schema_transforms::tree::{JsonObject, JsonTree};

mod tests;

const RATE_LIMIT_DESCRIPTION: &str = "\
Unified rate-limit error.

Every rate-limit condition surfaced by litellm \u{2014} whether it originated from
an upstream LLM provider, a vendor batch endpoint, or one of litellm's own
proxy-side limiters (parallel-requests, dynamic-rate, batch-rate, budget,
max-iterations, etc.) \u{2014} is raised as an instance of this class.

The :attr:`category` attribute lets callers distinguish the source. See
:class:`RateLimitErrorCategory` for the available values.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorStatus {
    pub status: u16,
    pub description: String,
}

// LITELLM_EXCEPTION_TYPES keeps a status's first position while the last class claiming it wins
#[must_use]
pub fn error_responses() -> Vec<ErrorStatus> {
    vec![
        ErrorStatus {
            status: 401,
            description: "AuthenticationError".to_owned(),
        },
        ErrorStatus {
            status: 404,
            description: "NotFoundError".to_owned(),
        },
        ErrorStatus {
            status: 400,
            description: "ContentPolicyViolationError".to_owned(),
        },
        ErrorStatus {
            status: 422,
            description: "UnprocessableEntityError".to_owned(),
        },
        ErrorStatus {
            status: 408,
            description: "Timeout".to_owned(),
        },
        ErrorStatus {
            status: 403,
            description: "PermissionDeniedError".to_owned(),
        },
        ErrorStatus {
            status: 429,
            description: RATE_LIMIT_DESCRIPTION.to_owned(),
        },
        ErrorStatus {
            status: 500,
            description: "JSONSchemaValidationError".to_owned(),
        },
        ErrorStatus {
            status: 503,
            description: "APIConnectionError".to_owned(),
        },
    ]
}

#[must_use]
pub fn error_response_schema() -> JsonObject {
    let example = JsonObject::from([(
        "error".to_owned(),
        JsonTree::Object(JsonObject::from([
            ("code".to_owned(), JsonTree::Text("error_code".to_owned())),
            (
                "message".to_owned(),
                JsonTree::Text("Error message".to_owned()),
            ),
            ("param".to_owned(), JsonTree::Text("error_param".to_owned())),
            ("type".to_owned(), JsonTree::Text("error_type".to_owned())),
        ])),
    )]);
    let detail = JsonObject::from([
        ("additionalProperties".to_owned(), JsonTree::Flag(true)),
        ("type".to_owned(), JsonTree::Text("object".to_owned())),
        ("title".to_owned(), JsonTree::Text("Detail".to_owned())),
        ("example".to_owned(), JsonTree::Object(example)),
    ]);
    JsonObject::from([
        (
            "properties".to_owned(),
            JsonTree::Object(JsonObject::from([(
                "detail".to_owned(),
                JsonTree::Object(detail),
            )])),
        ),
        ("type".to_owned(), JsonTree::Text("object".to_owned())),
        (
            "required".to_owned(),
            JsonTree::Array(vec![JsonTree::Text("detail".to_owned())]),
        ),
        (
            "title".to_owned(),
            JsonTree::Text("ErrorResponse".to_owned()),
        ),
    ])
}
