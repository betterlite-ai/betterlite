use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankRequest};
use indexmap::IndexMap;
use serde_json::Number;

// LiteLLM builds these dicts with literal constructors, so the signed body keeps their key order
fn object<const SIZE: usize>(entries: [(&str, Json); SIZE]) -> Json {
    Json::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

#[must_use]
pub fn transform_request(request: &RerankRequest) -> IndexMap<String, Json> {
    // absent, null and zero all fall back to the document count; any other value passes through
    let number_of_results = match request.top_n {
        Some(count) if count != 0 => count,
        Some(_) | None => i64::try_from(request.documents.len()).unwrap_or(i64::MAX),
    };
    let query = object([
        (
            "textQuery",
            object([("text", Json::Text(request.query.clone()))]),
        ),
        ("type", Json::Text("TEXT".to_owned())),
    ]);
    let configuration = object([
        (
            "bedrockRerankingConfiguration",
            object([
                (
                    "modelConfiguration",
                    object([("modelArn", Json::Text(request.model.clone()))]),
                ),
                (
                    "numberOfResults",
                    Json::Number(Number::from(number_of_results)),
                ),
            ]),
        ),
        ("type", Json::Text("BEDROCK_RERANKING_MODEL".to_owned())),
    ]);
    IndexMap::from([
        ("queries".to_owned(), Json::List(vec![query])),
        ("rerankingConfiguration".to_owned(), configuration),
        (
            "sources".to_owned(),
            Json::List(request.documents.iter().map(source).collect()),
        ),
    ])
}

fn source(document: &RerankDocument) -> Json {
    let inline = match document {
        RerankDocument::Text(text) => object([
            ("textDocument", object([("text", Json::Text(text.clone()))])),
            ("type", Json::Text("TEXT".to_owned())),
        ]),
        RerankDocument::Object(fields) => object([
            ("jsonDocument", Json::Object(fields.clone())),
            ("type", Json::Text("JSON".to_owned())),
        ]),
    };
    object([
        ("inlineDocumentSource", inline),
        ("type", Json::Text("INLINE".to_owned())),
    ])
}
