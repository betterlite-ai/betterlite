use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

const SAMPLE_TEXT: &str = "This is a sample search result from the vector store.";

// mock_vector_store_create_response: the caller's dict comes back unchanged, and a null one is LiteLLM's sample
#[must_use]
pub fn mock_create_response(mock_response: Option<&Json>) -> Json {
    match mock_response {
        Some(given) => given.clone(),
        None => Json::Object(IndexMap::from([
            ("id".to_owned(), Json::Text("vs_mock123".to_owned())),
            ("object".to_owned(), Json::Text("vector_store".to_owned())),
            (
                "created_at".to_owned(),
                Json::Number(Number::from(1699061776_u64)),
            ),
            (
                "name".to_owned(),
                Json::Text("Mock Vector Store".to_owned()),
            ),
            ("bytes".to_owned(), Json::Number(Number::from(0_u8))),
            (
                "file_counts".to_owned(),
                Json::Object(IndexMap::from([
                    ("in_progress".to_owned(), Json::Number(Number::from(0_u8))),
                    ("completed".to_owned(), Json::Number(Number::from(0_u8))),
                    ("failed".to_owned(), Json::Number(Number::from(0_u8))),
                    ("cancelled".to_owned(), Json::Number(Number::from(0_u8))),
                    ("total".to_owned(), Json::Number(Number::from(0_u8))),
                ])),
            ),
            ("status".to_owned(), Json::Text("completed".to_owned())),
            ("expires_after".to_owned(), Json::Null),
            ("expires_at".to_owned(), Json::Null),
            ("last_active_at".to_owned(), Json::Null),
            ("metadata".to_owned(), Json::Null),
        ])),
    }
}

// mock_vector_store_search_response: a list replaces data exactly, and LiteLLM's sample fills a null one
#[must_use]
pub fn mock_search_response(mock_results: Option<&Json>) -> Json {
    let data = match mock_results {
        Some(Json::List(items)) => Json::List(items.clone()),
        _ => Json::List(vec![Json::Object(IndexMap::from([
            (
                "score".to_owned(),
                Number::from_f64(0.95).map_or(Json::Null, Json::Number),
            ),
            (
                "content".to_owned(),
                Json::List(vec![Json::Object(IndexMap::from([
                    ("text".to_owned(), Json::Text(SAMPLE_TEXT.to_owned())),
                    ("type".to_owned(), Json::Text("text".to_owned())),
                ]))]),
            ),
        ]))]),
    };
    Json::Object(IndexMap::from([
        (
            "object".to_owned(),
            Json::Text("vector_store.search_results.page".to_owned()),
        ),
        (
            "search_query".to_owned(),
            Json::Text("sample query".to_owned()),
        ),
        ("data".to_owned(), data),
    ]))
}
