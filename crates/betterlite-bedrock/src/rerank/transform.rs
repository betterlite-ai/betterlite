use betterlite_models::decisions::Json;
use betterlite_openai::api::rerank_contracts::records::{RerankDocument, RerankRequest};
use indexmap::IndexMap;
use serde_json::Number;

// LiteLLM builds these dicts with literal constructors, so the signed body keeps their key order
#[must_use]
pub fn transform_request(request: &RerankRequest) -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    let mut text_query = IndexMap::new();
    text_query.insert("text".to_owned(), Json::Text(request.query.clone()));
    let mut query = IndexMap::new();
    query.insert("textQuery".to_owned(), Json::Object(text_query));
    query.insert("type".to_owned(), Json::Text("TEXT".to_owned()));
    body.insert("queries".to_owned(), Json::List(vec![Json::Object(query)]));
    body.insert(
        "rerankingConfiguration".to_owned(),
        Json::Object(reranking_configuration(request)),
    );
    body.insert(
        "sources".to_owned(),
        Json::List(request.documents.iter().map(source).collect()),
    );
    body
}

fn reranking_configuration(request: &RerankRequest) -> IndexMap<String, Json> {
    let mut model_configuration = IndexMap::new();
    model_configuration.insert("modelArn".to_owned(), Json::Text(request.model.clone()));
    let mut bedrock = IndexMap::new();
    bedrock.insert(
        "modelConfiguration".to_owned(),
        Json::Object(model_configuration),
    );
    bedrock.insert(
        "numberOfResults".to_owned(),
        Json::Number(Number::from(number_of_results(request))),
    );
    let mut configuration = IndexMap::new();
    configuration.insert(
        "bedrockRerankingConfiguration".to_owned(),
        Json::Object(bedrock),
    );
    configuration.insert(
        "type".to_owned(),
        Json::Text("BEDROCK_RERANKING_MODEL".to_owned()),
    );
    configuration
}

// absent, null and zero all fall back to the document count; any other value passes through
fn number_of_results(request: &RerankRequest) -> i64 {
    match request.top_n {
        Some(count) if count != 0 => count,
        Some(_) | None => i64::try_from(request.documents.len()).unwrap_or(i64::MAX),
    }
}

fn source(document: &RerankDocument) -> Json {
    let mut inline = IndexMap::new();
    match document {
        RerankDocument::Text(text) => {
            let mut text_document = IndexMap::new();
            text_document.insert("text".to_owned(), Json::Text(text.clone()));
            inline.insert("textDocument".to_owned(), Json::Object(text_document));
            inline.insert("type".to_owned(), Json::Text("TEXT".to_owned()));
        }
        RerankDocument::Object(fields) => {
            inline.insert("jsonDocument".to_owned(), Json::Object(fields.clone()));
            inline.insert("type".to_owned(), Json::Text("JSON".to_owned()));
        }
    }
    let mut inline_source = IndexMap::new();
    inline_source.insert("inlineDocumentSource".to_owned(), Json::Object(inline));
    inline_source.insert("type".to_owned(), Json::Text("INLINE".to_owned()));
    Json::Object(inline_source)
}
