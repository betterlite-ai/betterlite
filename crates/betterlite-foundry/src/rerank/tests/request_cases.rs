use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::RerankDocument;
use indexmap::IndexMap;

use crate::rerank::reply::transform_reply;

use super::support::{LIMITS, MULTILINGUAL, body, document, expected_body, hidden, request};

const WITH_DOCUMENT: &str =
    r#"{"results":[{"index":0,"relevance_score":0.5,"document":{"text":"first document"}}]}"#;
const WITHOUT_DOCUMENT: &str = r#"{"results":[{"index":0,"relevance_score":0.5}]}"#;

pub(super) fn query_documents_top_n() {
    minimal_documents();
    full_options();
    zero_and_empty();
}

fn minimal_documents() {
    let mut minimal = request();
    minimal.documents = vec![document("first document"), document("second document")];
    assert_eq!(
        body(&minimal),
        expected_body(
            r#"{"documents":["first document","second document"],"model":"cohere-rerank-v3-multilingual","query":"which document answers the question","return_documents":true}"#
        )
    );
}

fn full_options() {
    let mut options = request();
    options.documents = vec![
        RerankDocument::Object(IndexMap::from([(
            String::from("text"),
            Json::Text(String::from("first document")),
        )])),
        document("second document"),
    ];
    options.top_n = Some(1);
    options.rank_fields = Some(vec![String::from("text")]);
    options.max_chunks_per_doc = Some(2);
    options.return_documents = Some(false);
    assert_eq!(
        body(&options),
        expected_body(
            r#"{"documents":[{"text":"first document"},"second document"],"max_chunks_per_doc":2,"model":"cohere-rerank-v3-multilingual","query":"which document answers the question","rank_fields":["text"],"return_documents":false,"top_n":1}"#
        )
    );
}

fn zero_and_empty() {
    let mut zero = request();
    zero.top_n = Some(0);
    assert_eq!(
        body(&zero),
        expected_body(
            r#"{"documents":["first document"],"model":"cohere-rerank-v3-multilingual","query":"which document answers the question","return_documents":true,"top_n":0}"#
        )
    );
    let mut empty = request();
    empty.model = String::new();
    empty.query = String::new();
    empty.documents = Vec::new();
    assert_eq!(
        body(&empty),
        expected_body(r#"{"documents":[],"model":"","query":"","return_documents":true}"#)
    );
}

pub(super) fn return_documents() {
    request_default();
    reply_keeps_what_the_body_sent();
}

fn request_default() {
    let mut absent = request();
    absent.return_documents = None;
    assert_eq!(
        body(&absent).get("return_documents"),
        Some(&Json::Bool(true))
    );

    let mut explicit_true = request();
    explicit_true.return_documents = Some(true);
    assert_eq!(
        body(&explicit_true).get("return_documents"),
        Some(&Json::Bool(true))
    );

    let mut explicit_false = request();
    explicit_false.return_documents = Some(false);
    assert_eq!(
        body(&explicit_false).get("return_documents"),
        Some(&Json::Bool(false))
    );
}

// the provider decides whether a document comes back, so the reply keeps or omits it as the body sent it
fn reply_keeps_what_the_body_sent() {
    let with_document = transform_reply(
        200,
        WITH_DOCUMENT.as_bytes(),
        LIMITS,
        hidden(Some(MULTILINGUAL)),
    )
    .unwrap();
    assert_eq!(
        with_document
            .results
            .as_ref()
            .and_then(|results| results[0].document.as_ref())
            .map(|kept| kept.text.as_str()),
        Some("first document")
    );
    let without_document = transform_reply(
        200,
        WITHOUT_DOCUMENT.as_bytes(),
        LIMITS,
        hidden(Some(MULTILINGUAL)),
    )
    .unwrap();
    assert_eq!(
        without_document
            .results
            .as_ref()
            .map(|results| &results[0].document),
        Some(&None)
    );
}
