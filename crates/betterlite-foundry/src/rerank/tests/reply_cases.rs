use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::cohere_rerank::CohereRerankError;
use serde_json::Number;

use crate::rerank::error::FoundryRerankError;
use crate::rerank::reply::{
    FoundryRerankReply, HIDDEN_MODEL_PARAM, MODEL_GROUP_PARAM, transform_reply,
};

use super::support::{BODY, ENGLISH, LIMITS, MULTILINGUAL, hidden};

const SCORES: &str = r#"{"results":[{"index":0,"relevance_score":0.987654321,"document":{"text":"first document"}},{"index":1,"relevance_score":0.123456},{"index":2,"relevance_score":1},{"index":3,"relevance_score":0.1234567890123456789012},{"index":4,"relevance_score":0.0}]}"#;
const WIRE_MODEL: &str =
    r#"{"id":"rerank-id","results":[],"model":"azure_ai/cohere-rerank-v3-english"}"#;

pub(super) fn model_group_normalization() {
    assert_hidden_model(
        Some(MULTILINGUAL),
        &Json::Text(String::from("azure_ai/cohere-rerank-v3-multilingual")),
    );
    assert_hidden_model(
        Some(ENGLISH),
        &Json::Text(String::from("azure_ai/cohere-rerank-v3-english")),
    );
    assert_hidden_model(
        Some("offer-unknown-paygo"),
        &Json::Text(String::from("offer-unknown-paygo")),
    );
    assert_hidden_model(None, &Json::Null);

    // the public model field of a wire body never becomes the hidden model
    let no_group = transform_reply(200, WIRE_MODEL.as_bytes(), LIMITS, hidden(None)).unwrap();
    assert_eq!(
        no_group.hidden_params.get(HIDDEN_MODEL_PARAM),
        Some(&Json::Null)
    );

    // a group the conversion left as something other than text stays as it is
    let numeric = BTreeMap::from([(
        MODEL_GROUP_PARAM.to_owned(),
        Json::Number(Number::from(7_u8)),
    )]);
    let kept = transform_reply(200, BODY.as_bytes(), LIMITS, numeric).unwrap();
    assert_eq!(
        kept.hidden_params.get(HIDDEN_MODEL_PARAM),
        Some(&Json::Number(Number::from(7_u8)))
    );
}

fn assert_hidden_model(group: Option<&str>, expected: &Json) {
    let reply = transform_reply(200, BODY.as_bytes(), LIMITS, hidden(group)).unwrap();
    assert_eq!(reply.hidden_params.get(HIDDEN_MODEL_PARAM), Some(expected));
}

pub(super) fn relevance_scores() {
    let reply = transform_reply(200, SCORES.as_bytes(), LIMITS, hidden(None)).unwrap();
    let results = reply.results.expect("the fixture body has results");
    let scores: Vec<String> = results
        .iter()
        .map(|entry| entry.relevance_score.0.normalize().to_string())
        .collect();
    assert_eq!(
        scores,
        [
            "0.987654321",
            "0.123456",
            "1",
            "0.1234567890123456789012",
            "0"
        ]
    );
    assert_eq!(results[0].index, 0);
    assert_eq!(
        results[0].document.as_ref().map(|kept| kept.text.as_str()),
        Some("first document")
    );
    assert_eq!(results[1].document, None);
}

pub(super) fn rerank_error() {
    unparsable_bodies();
    wrong_shapes();
    json_bodies();
}

fn unparsable_bodies() {
    let not_json = transform_reply(502, b"upstream unavailable", LIMITS, hidden(None)).unwrap_err();
    assert!(matches!(
        &not_json,
        FoundryRerankError::Cohere(CohereRerankError::ResponseJson { status: 502, .. })
    ));
    assert!(not_json.to_string().contains("not JSON"));

    let empty = transform_reply(400, b"", LIMITS, hidden(None)).unwrap_err();
    assert!(matches!(
        &empty,
        FoundryRerankError::Cohere(CohereRerankError::ResponseJson { status: 400, .. })
    ));
}

fn wrong_shapes() {
    let array = transform_reply(200, b"[]", LIMITS, hidden(None)).unwrap_err();
    assert!(matches!(
        &array,
        FoundryRerankError::Cohere(CohereRerankError::Decode(_))
    ));

    let wrong_results =
        transform_reply(200, br#"{"results":"wrong"}"#, LIMITS, hidden(None)).unwrap_err();
    assert!(matches!(
        &wrong_results,
        FoundryRerankError::Cohere(CohereRerankError::Decode(_))
    ));
}

fn json_bodies() {
    // LiteLLM never reads the HTTP status, so a JSON body on a 500 still becomes a reply
    let server_error = transform_reply(500, BODY.as_bytes(), LIMITS, hidden(None)).unwrap();
    assert_eq!(server_error.id.as_deref(), Some("rerank-id"));
    // the upstream meta block, the billed units included, comes back as the provider wrote it
    assert_eq!(
        serde_json::to_string(&server_error.meta.expect("the fixture body has meta")).unwrap(),
        r#"{"api_version":{"version":"1"},"billed_units":{"search_units":1}}"#
    );

    // an error-shaped body validates as an empty reply, which is what LiteLLM returns there too
    let error_body = transform_reply(
        429,
        br#"{"error":{"message":"slow down"}}"#,
        LIMITS,
        hidden(None),
    )
    .unwrap();
    assert_eq!(
        error_body,
        FoundryRerankReply {
            id: None,
            results: None,
            meta: None,
            hidden_params: BTreeMap::from([(HIDDEN_MODEL_PARAM.to_owned(), Json::Null)]),
        }
    );
}
