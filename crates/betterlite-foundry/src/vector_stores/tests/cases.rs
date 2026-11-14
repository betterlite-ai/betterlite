use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_store_records::records::vector::{
    VectorStoreIndexEndpoints, VectorStoreSearchResponse,
};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};

use crate::vector_stores::create::{CreateStep, create_vector_store};
use crate::vector_stores::embedding::{query_embedding_configuration, query_embedding_model};
use crate::vector_stores::environment::{
    EnvironmentRequest, Headers, Params, auth_credentials, complete_url, endpoints_by_type,
    validate_environment,
};
use crate::vector_stores::error::AzureSearchError;
use crate::vector_stores::reply::{
    AzureSearchStoreReply, SEARCH_RESULTS_OBJECT, decode_azure_search_reply,
};
use crate::vector_stores::search::{
    AzureSearchStoreRequest, SearchCall, SearchLogging, SearchQuery, asearch_request,
    search_request,
};

use super::support::{Call, FixtureExecutor, NoSources, empty_credentials};

const API_BASE: &str = "https://foundry-search.example";
const EMBEDDING_MODEL: &str = "openai/text-embedding-3-small";

#[rstest]
#[case::store_type_endpoints(0)]
#[case::auth_credentials(1)]
#[case::search_query_fields(2)]
#[case::vector_query_with_embedding(3)]
#[case::document_score_citation(4)]
#[case::unsupported_store_create(5)]
#[case::azure_search_endpoint(6)]
fn vector_stores_cases(#[case] case: u8) {
    match case {
        0 => store_type_endpoints(),
        1 => auth_credentials_case(),
        2 => search_query_fields(),
        3 => vector_query_with_embedding(),
        4 => document_score_citation(),
        5 => unsupported_store_create(),
        _ => azure_search_endpoint(),
    }
}

fn params(text: &str) -> Params {
    serde_json::from_str(text).expect("fixture params")
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn embedding_params() -> Params {
    params(&format!(
        r#"{{"litellm_embedding_model":"{EMBEDDING_MODEL}"}}"#
    ))
}

fn secret(headers: &Headers, name: &str) -> String {
    headers
        .get(name)
        .expect("header")
        .expose_secret()
        .to_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Permission {
    Read,
    Write,
}

fn route_matches(pattern: &str, path: &str) -> bool {
    if path.contains(pattern) {
        return true;
    }
    match pattern.split_once('{') {
        Some((prefix, _)) => !prefix.is_empty() && path.contains(prefix),
        None => false,
    }
}

fn any_route(routes: Option<&Vec<(String, String)>>, method: &str, path: &str) -> bool {
    routes.is_some_and(|routes| {
        routes.iter().any(|(route_method, route_path)| {
            route_method == method && route_matches(route_path, path)
        })
    })
}

// the consumer checks the write map before the read map, so a path matching both demands the stronger grant
fn classify(endpoints: &VectorStoreIndexEndpoints, method: &str, path: &str) -> Option<Permission> {
    if any_route(endpoints.write.as_ref(), method, path) {
        return Some(Permission::Write);
    }
    any_route(endpoints.read.as_ref(), method, path).then_some(Permission::Read)
}

fn pairs(routes: &[(&str, &str)]) -> Vec<(String, String)> {
    routes
        .iter()
        .map(|(method, path)| ((*method).to_owned(), (*path).to_owned()))
        .collect()
}

fn store_type_endpoints() {
    let endpoints = endpoints_by_type();
    assert_eq!(
        endpoints.read,
        Some(pairs(&[
            ("GET", "/indexes/"),
            ("POST", "/docs/search"),
            ("POST", "/docs/suggest"),
            ("POST", "/docs/autocomplete"),
            ("POST", "/analyze"),
        ]))
    );
    assert_eq!(endpoints.write, Some(pairs(&[("POST", "/docs/index")])));
    let routes = endpoints
        .read
        .iter()
        .flatten()
        .chain(endpoints.write.iter().flatten());
    assert!(routes.all(|(_, path)| !path.contains('{')));
    store_type_endpoints_classifies_reads();
    store_type_endpoints_write_first();
    store_type_endpoints_keeps_methods_apart();
}

fn store_type_endpoints_classifies_reads() {
    let endpoints = endpoints_by_type();
    assert_eq!(
        classify(&endpoints, "GET", "/indexes/analyze-index/stats"),
        Some(Permission::Read)
    );
    assert_eq!(
        classify(&endpoints, "POST", "/indexes/analyze-index/docs/search"),
        Some(Permission::Read)
    );
    assert_eq!(
        classify(&endpoints, "POST", "/indexes/analyze-index/docs/suggest"),
        Some(Permission::Read)
    );
    assert_eq!(
        classify(
            &endpoints,
            "POST",
            "/indexes/analyze-index/docs/autocomplete"
        ),
        Some(Permission::Read)
    );
    assert_eq!(
        classify(&endpoints, "POST", "/indexes/analyze-index/analyze"),
        Some(Permission::Read)
    );
    assert_eq!(
        classify(&endpoints, "DELETE", "/indexes/analyze-index"),
        None
    );
}

// an index named analyze puts the read fragment inside the batch-write path, which write-first must not shadow
fn store_type_endpoints_write_first() {
    let endpoints = endpoints_by_type();
    let batch = "/indexes/analyze-index/docs/index";
    assert!(any_route(endpoints.read.as_ref(), "POST", batch));
    assert_eq!(classify(&endpoints, "POST", batch), Some(Permission::Write));
}

fn store_type_endpoints_keeps_methods_apart() {
    let endpoints = endpoints_by_type();
    assert_eq!(classify(&endpoints, "GET", "/docs/search"), None);
    assert_eq!(classify(&endpoints, "POST", "/indexes/"), None);
}

fn auth_credentials_case() {
    let keyed = auth_credentials(&params(r#"{"api_key":"fixture-key"}"#)).expect("keyed");
    assert_eq!(secret(&keyed.headers, "api-key"), "fixture-key");
    let empty = auth_credentials(&params(r#"{"api_key":""}"#)).expect("empty key");
    assert_eq!(secret(&empty.headers, "api-key"), "");
    assert!(matches!(
        auth_credentials(&Params::new()),
        Err(AzureSearchError::MissingApiKey)
    ));
    assert!(matches!(
        auth_credentials(&params(r#"{"api_key":null}"#)),
        Err(AzureSearchError::MissingApiKey)
    ));
    assert!(matches!(
        auth_credentials(&params(r#"{"api_key":1}"#)),
        Err(AzureSearchError::InvalidField {
            field: "api_key",
            ..
        })
    ));
    validate_environment_case();
    validate_environment_overwrites_content_type();
}

fn validate_environment_case() {
    let credentials = empty_credentials();
    let sources = NoSources;
    let key = SecretString::from("fixture-key");
    let existing = validate_environment(EnvironmentRequest {
        headers: Headers::from([("api-key".to_owned(), SecretString::from("fixture-existing"))]),
        api_key: Some(&key),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .expect("headers");
    assert_eq!(secret(&existing, "api-key"), "fixture-existing");
    assert_eq!(secret(&existing, "Content-Type"), "application/json");
    let added = validate_environment(EnvironmentRequest {
        headers: Headers::new(),
        api_key: Some(&key),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .expect("headers");
    assert_eq!(secret(&added, "api-key"), "fixture-key");
    let tokenless = validate_environment(EnvironmentRequest {
        headers: Headers::new(),
        api_key: None,
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .expect("headers");
    assert!(!tokenless.contains_key("api-key"));
    assert_eq!(secret(&tokenless, "Content-Type"), "application/json");
}

fn validate_environment_overwrites_content_type() {
    let credentials = empty_credentials();
    let sources = NoSources;
    let key = SecretString::from("fixture-key");
    let headers = Headers::from([("Content-Type".to_owned(), SecretString::from("text/plain"))]);
    let validated = validate_environment(EnvironmentRequest {
        headers,
        api_key: Some(&key),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .expect("headers");
    assert_eq!(secret(&validated, "Content-Type"), "application/json");
    let cased = Headers::from([("content-type".to_owned(), SecretString::from("text/plain"))]);
    let validated = validate_environment(EnvironmentRequest {
        headers: cased,
        api_key: Some(&key),
        credentials: &credentials,
        refresh_enabled: false,
        provider: None,
        sources: &sources,
    })
    .expect("headers");
    assert_eq!(secret(&validated, "content-type"), "text/plain");
    assert_eq!(secret(&validated, "Content-Type"), "application/json");
}

fn search_call(
    query: &SearchQuery,
    optional: &Params,
    litellm_params: &Params,
    executor: &FixtureExecutor,
) -> (Result<SearchCall, AzureSearchError>, SearchLogging) {
    let mut logging = SearchLogging::default();
    let call = search_request(AzureSearchStoreRequest {
        vector_store_id: "store-1",
        query,
        optional_params: optional,
        api_base: API_BASE,
        litellm_params,
        logging: &mut logging,
        embedding_executor: executor,
    });
    (call, logging)
}

fn search_query_fields() {
    let executor = FixtureExecutor::new(vec![0.25, 0.5]);
    let litellm_params = embedding_params();
    let query = SearchQuery::Text("hello".to_owned());
    let (call, logging) = search_call(&query, &Params::new(), &litellm_params, &executor);
    let call = call.expect("search");
    assert_eq!(
        call.url,
        "https://foundry-search.example/indexes/store-1/docs/search?api-version=2024-07-01"
    );
    assert_eq!(call.body.search, "*");
    assert_eq!(call.body.select, "id,content");
    assert_eq!(call.body.top, 10);
    let vector_query = &call.body.vector_queries[0];
    assert_eq!(vector_query.vector, vec![0.25_f64, 0.5_f64]);
    assert_eq!(vector_query.fields, "contentVector");
    assert_eq!(vector_query.kind, "vector");
    assert_eq!(vector_query.top_k, 10);
    assert_eq!(
        logging.model_call_details.get("input"),
        Some(&Json::Text("hello".to_owned()))
    );
    assert_eq!(
        logging.model_call_details.get("embedding_model"),
        Some(&Json::Text(EMBEDDING_MODEL.to_owned()))
    );
    assert_eq!(logging.model_call_details.get("top_k"), Some(&number(10)));
    let calls: Vec<Call> = executor.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].model, EMBEDDING_MODEL);
    assert_eq!(calls[0].query, "hello");
    assert!(calls[0].configuration.is_empty());
}

fn search_query_fields_options() {
    let executor = FixtureExecutor::new(vec![0.5]);
    let litellm_params = params(&format!(
        r#"{{"litellm_embedding_model":"{EMBEDDING_MODEL}","azure_search_vector_field":"customVector","litellm_embedding_config":{{"dimensions":2}}}}"#
    ));
    let optional = params(r#"{"top_k":3,"filters":{"genre":"x"},"max_num_results":1}"#);
    let query = SearchQuery::Sequence(vec!["first".to_owned(), String::new(), "third".to_owned()]);
    let mut logging = SearchLogging::default();
    let call = search_request(AzureSearchStoreRequest {
        vector_store_id: "store/with?literal",
        query: &query,
        optional_params: &optional,
        api_base: API_BASE,
        litellm_params: &litellm_params,
        logging: &mut logging,
        embedding_executor: &executor,
    })
    .expect("search");
    assert_eq!(call.body.top, 3);
    assert_eq!(call.body.vector_queries[0].top_k, 3);
    assert_eq!(call.body.vector_queries[0].fields, "customVector");
    assert!(call.url.contains("/indexes/store/with?literal/docs/search"));
    assert_eq!(executor.calls()[0].query, "first  third");
    assert_eq!(
        executor.calls()[0].configuration.get("dimensions"),
        Some(&number(2))
    );
}

// a default applies only when the key is absent, so an explicit empty vector field stays empty
fn search_query_fields_defaults_only_when_absent() {
    let executor = FixtureExecutor::new(vec![0.5]);
    let litellm_params = params(&format!(
        r#"{{"litellm_embedding_model":"{EMBEDDING_MODEL}","azure_search_vector_field":""}}"#
    ));
    let optional = params(r#"{"top_k":null}"#);
    let query = SearchQuery::Text("q".to_owned());
    let (call, logging) = search_call(&query, &optional, &litellm_params, &executor);
    let call = call.expect("search");
    assert_eq!(call.body.vector_queries[0].fields, "");
    assert_eq!(call.body.top, 10);
    assert_eq!(logging.model_call_details.get("top_k"), Some(&number(10)));
}

// the embedding runs before the body is built, so a bad top_k still reaches the executor once
fn search_query_fields_rejects_bad_types() {
    let executor = FixtureExecutor::new(vec![0.5]);
    let litellm_params = embedding_params();
    let query = SearchQuery::Text("q".to_owned());
    let optional = params(r#"{"top_k":"three"}"#);
    let (call, _logging) = search_call(&query, &optional, &litellm_params, &executor);
    assert!(matches!(
        call,
        Err(AzureSearchError::InvalidField { field: "top_k", .. })
    ));
    assert_eq!(executor.calls().len(), 1);
    let (call, _logging) = search_call(&query, &Params::new(), &params("{}"), &executor);
    assert!(matches!(call, Err(AzureSearchError::MissingEmbeddingModel)));
    assert_eq!(executor.calls().len(), 1);
}

fn search_query_fields_embedding_configuration() {
    let with_object = params(r#"{"litellm_embedding_config":{"dimensions":2}}"#);
    assert_eq!(
        query_embedding_configuration(&with_object).get("dimensions"),
        Some(&number(2))
    );
    let with_list = params(r#"{"litellm_embedding_config":[1]}"#);
    assert!(query_embedding_configuration(&with_list).is_empty());
    assert!(query_embedding_model(&params("{}")).is_err());
    assert!(query_embedding_model(&params(r#"{"litellm_embedding_model":""}"#)).is_err());
    assert_eq!(
        query_embedding_model(&params(r#"{"litellm_embedding_model":"m"}"#)).expect("model"),
        "m"
    );
}

fn vector_query_with_embedding() {
    let executor = FixtureExecutor::new(vec![0.5, 0.25, 0.125]);
    let litellm_params = embedding_params();
    let query = SearchQuery::Text("ordered".to_owned());
    let mut logging = SearchLogging::default();
    let call = search_request(AzureSearchStoreRequest {
        vector_store_id: "store-2",
        query: &query,
        optional_params: &Params::new(),
        api_base: API_BASE,
        litellm_params: &litellm_params,
        logging: &mut logging,
        embedding_executor: &executor,
    })
    .expect("search");
    assert_eq!(
        call.body.vector_queries[0].vector,
        vec![0.5_f64, 0.25_f64, 0.125_f64]
    );
    assert_eq!(
        call.url,
        "https://foundry-search.example/indexes/store-2/docs/search?api-version=2024-07-01"
    );
    vector_query_with_embedding_async();
}

fn vector_query_with_embedding_async() {
    let executor = FixtureExecutor::new(vec![0.125, 0.5]);
    let litellm_params = embedding_params();
    let query = SearchQuery::Sequence(vec!["async".to_owned(), "query".to_owned()]);
    let mut logging = SearchLogging::default();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");
    let call = runtime
        .block_on(asearch_request(AzureSearchStoreRequest {
            vector_store_id: "async-store",
            query: &query,
            optional_params: &Params::new(),
            api_base: API_BASE,
            litellm_params: &litellm_params,
            logging: &mut logging,
            embedding_executor: &executor,
        }))
        .expect("async search");
    assert_eq!(call.body.vector_queries[0].vector, vec![0.125_f64, 0.5_f64]);
    assert_eq!(executor.calls()[0].query, "async query");
}

fn decode_with_logging(
    body: &str,
    logging: &SearchLogging,
) -> Result<VectorStoreSearchResponse, AzureSearchError> {
    decode_azure_search_reply(&AzureSearchStoreReply {
        status_code: 200,
        headers: &[],
        body: body.as_bytes(),
        logging,
    })
}

fn decode(body: &str) -> Result<VectorStoreSearchResponse, AzureSearchError> {
    let logging = SearchLogging::default();
    decode_with_logging(body, &logging)
}

fn document_score_citation() {
    let body = r#"{"value":[{"id":"doc-1","content":"first text","@search.score":0.95,"title":"a","contentVector":[1]},{"id":"doc-2","content":"second"}]}"#;
    let page = decode(body).expect("page");
    assert_eq!(page.object.as_deref(), Some(SEARCH_RESULTS_OBJECT));
    assert_eq!(page.search_query.as_deref(), Some(""));
    let data = page.data.expect("data");
    assert_eq!(data.len(), 2);
    assert_eq!(data[0].file_id.as_deref(), Some("doc-1"));
    assert_eq!(data[0].filename.as_deref(), Some("Document doc-1"));
    assert_eq!(
        data[0].score.as_ref().map(|score| score.0),
        Some(Decimal::new(95, 2))
    );
    let content = data[0].content.as_ref().expect("content");
    assert_eq!(content[0].text.as_deref(), Some("first text"));
    assert_eq!(content[0].type_name.as_deref(), Some("text"));
    assert_eq!(
        data[1].score.as_ref().map(|score| score.0),
        Some(Decimal::ZERO)
    );
    document_score_citation_attributes();
    document_score_citation_defaults();
    document_score_citation_omits_missing_value();
    document_score_citation_uses_logged_query();
    document_score_citation_reports_provider_failure();
    document_score_citation_rejects_bad_shapes();
}

fn document_score_citation_attributes() {
    let body = r#"{"value":[{"id":"doc-1","content":"text","@search.score":0.5,"contentVector":[1],"custom_vector":[2],"title":"a","document_id":"old"}]}"#;
    let page = decode(body).expect("page");
    let data = page.data.expect("data");
    let attributes = data[0].attributes.as_ref().expect("attributes");
    assert_eq!(
        attributes.get("title").and_then(|value| value.as_str()),
        Some("a")
    );
    assert_eq!(
        attributes
            .get("custom_vector")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        attributes
            .get("document_id")
            .and_then(|value| value.as_str()),
        Some("doc-1")
    );
    assert!(attributes.get("id").is_none());
    assert!(attributes.get("content").is_none());
    assert!(attributes.get("contentVector").is_none());
    assert!(attributes.get("@search.score").is_none());
}

// an absent or null field takes its default instead of failing the whole page
fn document_score_citation_defaults() {
    let page = decode(r#"{"value":[{}]}"#).expect("page");
    let data = page.data.expect("data");
    assert_eq!(data[0].file_id.as_deref(), Some(""));
    assert_eq!(data[0].filename.as_deref(), Some("Document "));
    assert_eq!(
        data[0].score.as_ref().map(|score| score.0),
        Some(Decimal::ZERO)
    );
    let page =
        decode(r#"{"value":[{"id":null,"content":null,"@search.score":null}]}"#).expect("page");
    assert_eq!(page.data.expect("data")[0].file_id.as_deref(), Some(""));
}

fn document_score_citation_omits_missing_value() {
    let page = decode("{}").expect("page");
    assert!(page.data.expect("data").is_empty());
    assert_eq!(page.object.as_deref(), Some(SEARCH_RESULTS_OBJECT));
}

fn document_score_citation_uses_logged_query() {
    let executor = FixtureExecutor::new(vec![0.5]);
    let litellm_params = embedding_params();
    let query = SearchQuery::Text("cited query".to_owned());
    let (call, logging) = search_call(&query, &Params::new(), &litellm_params, &executor);
    assert!(call.is_ok());
    let page = decode_with_logging(r#"{"value":[]}"#, &logging).expect("page");
    assert_eq!(page.search_query.as_deref(), Some("cited query"));
}

fn document_score_citation_reports_provider_failure() {
    let logging = SearchLogging::default();
    let headers = [("x-test".to_owned(), "present".to_owned())];
    let reply = AzureSearchStoreReply {
        status_code: 200,
        headers: &headers,
        body: b"{",
        logging: &logging,
    };
    match decode_azure_search_reply(&reply) {
        Err(AzureSearchError::ReplyJson {
            status_code,
            headers: kept,
            ..
        }) => {
            assert_eq!(status_code, 200);
            assert_eq!(kept, headers);
        }
        other => panic!("unexpected outcome: {other:?}"),
    }
}

fn document_score_citation_rejects_bad_shapes() {
    assert!(matches!(
        decode("[]"),
        Err(AzureSearchError::ReplyShape { .. })
    ));
    assert!(matches!(
        decode(r#"{"value":null}"#),
        Err(AzureSearchError::ReplyShape { .. })
    ));
    assert!(matches!(
        decode(r#"{"value":["x"]}"#),
        Err(AzureSearchError::ReplyShape { .. })
    ));
    assert!(matches!(
        decode(r#"{"value":[{"id":1}]}"#),
        Err(AzureSearchError::ReplyShape { .. })
    ));
    assert!(matches!(
        decode(r#"{"value":[{"@search.score":"high"}]}"#),
        Err(AzureSearchError::ReplyShape { .. })
    ));
}

fn unsupported_store_create() {
    assert!(matches!(
        create_vector_store(CreateStep::Request),
        Err(AzureSearchError::CreateNotImplemented)
    ));
    assert!(matches!(
        create_vector_store(CreateStep::Response),
        Err(AzureSearchError::CreateNotImplemented)
    ));
}

fn azure_search_endpoint() {
    let service = r#"{"azure_search_service_name":"foundry-service"}"#;
    assert_eq!(
        complete_url(Some("https://custom.example///"), &params("{}")).expect("url"),
        "https://custom.example"
    );
    assert_eq!(
        complete_url(Some(""), &params(service)).expect("url"),
        "https://foundry-service.search.windows.net"
    );
    assert_eq!(
        complete_url(None, &params(service)).expect("url"),
        "https://foundry-service.search.windows.net"
    );
    azure_search_endpoint_rejects_missing_names();
    azure_search_endpoint_keeps_the_name_literal();
}

fn azure_search_endpoint_rejects_missing_names() {
    assert!(matches!(
        complete_url(None, &params("{}")),
        Err(AzureSearchError::MissingServiceName)
    ));
    assert!(matches!(
        complete_url(None, &params(r#"{"azure_search_service_name":null}"#)),
        Err(AzureSearchError::MissingServiceName)
    ));
    assert!(matches!(
        complete_url(None, &params(r#"{"azure_search_service_name":""}"#)),
        Err(AzureSearchError::MissingServiceName)
    ));
    assert!(matches!(
        complete_url(None, &params(r#"{"azure_search_service_name":1}"#)),
        Err(AzureSearchError::InvalidField {
            field: "azure_search_service_name",
            ..
        })
    ));
}

fn azure_search_endpoint_keeps_the_name_literal() {
    let litellm_params = params(r#"{"azure_search_service_name":"a b/c"}"#);
    assert_eq!(
        complete_url(None, &litellm_params).expect("url"),
        "https://a b/c.search.windows.net"
    );
}
