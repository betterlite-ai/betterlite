use std::collections::BTreeMap;

use betterlite_config::sections::openai_credentials::Settings;
use betterlite_openai::api::vector_store_records::records::basic::VectorStoreStatus;
use betterlite_openai::api::vector_store_records::records::union::VectorStoreRecord;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::error::VectorStoreError;
use betterlite_openai::vector_stores::files::{
    Attributes, FileCreateParams, FileListParams, FileOperation, FileUpdateParams,
};
use betterlite_openai::vector_stores::reply::{ReplyKind, VectorReply};
use betterlite_openai::vector_stores::store::{CreateParams, Query, SearchParams};
use betterlite_openai::vector_stores::{Headers, Request};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::fixture::{assert_json, parse};
use crate::openai_resource_dispatch::vector_stores::{
    Call, Dispatch, EndpointKind, Outcome, RequestCall,
};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://h.example/v1/vector_stores";

fn key(text: &str) -> SecretString {
    SecretString::from(text)
}

fn globals(api_key: Option<&str>) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: None,
        api_key: api_key.map(key),
        openai_key: None,
        organization: None,
    }
}

fn plain(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, secret)| (name.as_str(), secret.expose_secret()))
        .collect()
}

async fn run(
    call: Call<'_>,
    globals: LiteLlmGlobals,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let settings = Settings {
        api_base: String::from("https://env.example/v1"),
        organization: None,
    };
    let operation = OpenAiResourceOperation::VectorStores(Box::new(Dispatch {
        settings: &settings,
        globals,
        env_api_key: None,
        call,
    }));
    dispatch(operation).await
}

async fn outcome(call: Call<'_>, globals: LiteLlmGlobals) -> Outcome {
    let result = run(call, globals).await;
    let Ok(OpenAiResourceResult::VectorStores(done)) = result else {
        panic!("the call succeeds through the vector store leaf");
    };
    done
}

async fn leaf_error(call: Call<'_>) -> VectorStoreError {
    let result = run(call, globals(None)).await;
    let Err(OpenAiResourceError::VectorStores(error)) = result else {
        panic!("the call is a vector store leaf error");
    };
    error
}

async fn headers_of(call: Call<'_>, globals: LiteLlmGlobals) -> Headers {
    let Outcome::Headers(found) = outcome(call, globals).await else {
        panic!("the call returns headers");
    };
    found
}

async fn url_of(api_base: Option<&str>, litellm: Option<&str>, id: Option<&str>) -> String {
    let call = Call::Url {
        api_base,
        litellm_api_base: litellm,
        vector_store_id: id,
    };
    let Outcome::Url(found) = outcome(call, globals(None)).await else {
        panic!("the call returns its url");
    };
    found
}

async fn request_of(call: RequestCall<'_>) -> Request {
    let Outcome::Request(found) = outcome(Call::Request(call), globals(None)).await else {
        panic!("the call returns a request");
    };
    found
}

async fn reply_of(
    kind: ReplyKind,
    status: u16,
    body: &str,
) -> Result<VectorReply, VectorStoreError> {
    let response = reply(status, body);
    let call = Call::Reply {
        kind,
        response: &response,
    };
    match run(call, globals(None)).await {
        Ok(OpenAiResourceResult::VectorStores(Outcome::Reply(decoded))) => Ok(decoded),
        Err(OpenAiResourceError::VectorStores(error)) => Err(error),
        _ => panic!("a reply call returns a reply or a leaf error"),
    }
}

fn reply(status: u16, body: &str) -> http::Response<Vec<u8>> {
    let built = http::Response::builder().status(status);
    built.body(body.as_bytes().to_vec()).unwrap()
}

async fn credential_rows() {
    let given = key("sk-vs");
    let found = headers_of(Call::Auth(Some(&given)), globals(None)).await;
    assert_eq!(
        plain(&found),
        BTreeMap::from([("Authorization", "Bearer sk-vs")])
    );
    let missing = leaf_error(Call::Auth(None)).await;
    assert!(matches!(missing, VectorStoreError::MissingApiKey));
}

async fn environment_rows() {
    let added = Call::Environment {
        headers: Headers::new(),
        explicit_key: Some(key("p")),
    };
    let found = headers_of(added, globals(None)).await;
    assert_eq!(
        plain(&found),
        BTreeMap::from([
            ("Authorization", "Bearer p"),
            ("Content-Type", "application/json"),
            ("OpenAI-Beta", "assistants=v2"),
        ])
    );
    let kept = Call::Environment {
        headers: Headers::from([(String::from("OpenAI-Beta"), key(""))]),
        explicit_key: None,
    };
    let preserved = headers_of(kept, globals(Some("g"))).await;
    assert_eq!(plain(&preserved)["OpenAI-Beta"], "");
    assert_eq!(plain(&preserved)["Authorization"], "Bearer g");
}

async fn url_rows() {
    let absolute = url_of(Some("https://a.example/"), None, None).await;
    assert_eq!(absolute, "https://a.example/vector_stores");
    let trimmed = url_of(None, Some("https://g.example///"), Some("vs_1")).await;
    assert_eq!(trimmed, "https://g.example/vector_stores/vs_1/files");
    let encoded = url_of(None, None, Some("a/b")).await;
    assert_eq!(encoded, "https://env.example/v1/vector_stores/a%2Fb/files");
    let call = Call::Url {
        api_base: None,
        litellm_api_base: None,
        vector_store_id: Some(".."),
    };
    assert!(matches!(
        leaf_error(call).await,
        VectorStoreError::Segment(_)
    ));
}

async fn endpoint_rows() {
    let Outcome::Endpoints(store) =
        outcome(Call::Endpoints(EndpointKind::Store), globals(None)).await
    else {
        panic!("the store endpoints call returns endpoints");
    };
    let read = vec![(
        String::from("GET"),
        String::from("/vector_stores/{index_name}/search"),
    )];
    assert_eq!(store.read, Some(read));
    let write = vec![(String::from("POST"), String::from("/vector_stores"))];
    assert_eq!(store.write, Some(write));
    let Outcome::Endpoints(attached) =
        outcome(Call::Endpoints(EndpointKind::File), globals(None)).await
    else {
        panic!("the file endpoints call returns endpoints");
    };
    assert_eq!(attached.read.as_ref().map(Vec::len), Some(3));
    assert_eq!(attached.write.as_ref().map(Vec::len), Some(3));
}

async fn search_request_row() {
    let terms = Query::Many(vec![String::from("a"), String::from("b")]);
    let params = SearchParams {
        filters: Some(parse(r#"{"type":"eq"}"#)),
        max_num_results: Some(5),
        ranking_options: Some(parse(r#"{"ranker":"auto"}"#)),
        rewrite_query: Some(false),
    };
    let base = "https://h.example/v1/vector_stores?api-version=1";
    let found = request_of(RequestCall::Search {
        vector_store_id: "a/b c",
        query: &terms,
        params: &params,
        api_base: base,
    })
    .await;
    assert_eq!(
        found.url,
        "https://h.example/v1/vector_stores/a%2Fb%20c/search?api-version=1"
    );
    let sent = r#"{"query":["a","b"],"filters":{"type":"eq"},"max_num_results":5,"ranking_options":{"ranker":"auto"},"rewrite_query":false}"#;
    assert_json(&found.body, sent);
}

async fn create_request_rows() {
    let mut metadata = IndexMap::new();
    for index in 0..20_u8 {
        metadata.insert(format!("k{index}"), parse("\"v\""));
    }
    metadata.insert(String::from("hidden_params"), parse("\"x\""));
    metadata.shift_insert(1, String::from("count"), parse("3"));
    let params = CreateParams {
        name: Some(String::from("docs")),
        file_ids: Some(vec![String::from("f1")]),
        expires_after: None,
        chunking_strategy: None,
        metadata: Some(metadata),
    };
    let created = request_of(RequestCall::Create {
        params: &params,
        api_base: API_BASE,
    })
    .await;
    assert_eq!(created.url, API_BASE);
    let sent = created.body["metadata"].as_object().unwrap();
    assert_eq!(sent.len(), 16);
    assert!(sent.contains_key("k15") && !sent.contains_key("k16") && !sent.contains_key("count"));
    let empty = CreateParams::default();
    let defaults = request_of(RequestCall::Create {
        params: &empty,
        api_base: API_BASE,
    })
    .await;
    let nulls = r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#;
    assert_json(&defaults.body, nulls);
}

async fn attached_file_request_rows() {
    let attributes = IndexMap::from([
        (String::from("team"), parse("\"a\"")),
        (String::from("rank"), parse("2")),
    ]);
    let attach = FileCreateParams {
        file_id: Some(String::from("file_1")),
        attributes: Some(Attributes::Map(attributes)),
        chunking_strategy: None,
    };
    let attached = request_of(RequestCall::FileCreate {
        params: &attach,
        api_base: API_BASE,
    })
    .await;
    assert_eq!(attached.url, API_BASE);
    let sent = r#"{"file_id":"file_1","attributes":{"team":"a"}}"#;
    assert_json(&attached.body, sent);
    let paging = FileListParams {
        after: Some(String::from("file_9")),
        before: None,
        filter: None,
        limit: Some(10),
        order: Some(String::from("desc")),
    };
    let listed = request_of(RequestCall::FileList {
        params: &paging,
        api_base: API_BASE,
    })
    .await;
    assert_eq!(listed.url, API_BASE);
    let paged = r#"{"after":"file_9","limit":10,"order":"desc"}"#;
    assert_json(&listed.body, paged);
}

async fn file_operation_rows() {
    let cases = [
        (FileOperation::Retrieve, "/f%2F1"),
        (FileOperation::Content, "/f%2F1/content"),
        (FileOperation::Delete, "/f%2F1"),
    ];
    for (operation, suffix) in cases {
        let found = request_of(RequestCall::File {
            operation: &operation,
            file_id: "f/1",
            api_base: API_BASE,
        })
        .await;
        assert_eq!(found.url, format!("{API_BASE}{suffix}"));
        assert_json(&found.body, "{}");
    }
    let update = FileOperation::Update(FileUpdateParams {
        attributes: Attributes::Other(parse("7")),
    });
    let updated = request_of(RequestCall::File {
        operation: &update,
        file_id: "f1",
        api_base: API_BASE,
    })
    .await;
    assert_json(&updated.body, r#"{"attributes":7}"#);
}

async fn file_operation_error_row() {
    let call = Call::Request(RequestCall::File {
        operation: &FileOperation::Retrieve,
        file_id: "..",
        api_base: API_BASE,
    });
    assert!(matches!(
        leaf_error(call).await,
        VectorStoreError::Segment(_)
    ));
}

async fn reply_rows() {
    let search = r#"{"object":"vector_store.search_results.page","search_query":"q","data":[{"content":[{"text":"t","type":"text"}],"file_id":"f","filename":"a.txt","attributes":{"k":"v"}}]}"#;
    let Ok(VectorReply::Record(found)) = reply_of(ReplyKind::Search, 200, search).await else {
        panic!("the search reply decodes to a record");
    };
    let VectorStoreRecord::SearchResponse(page) = &*found else {
        panic!("a search reply is a search response record");
    };
    assert_eq!(page.search_query.as_deref(), Some("q"));
    let file_ids: Vec<_> = page
        .data
        .iter()
        .flatten()
        .map(|hit| hit.file_id.as_deref())
        .collect();
    assert_eq!(file_ids, [Some("f")]);
    assert_json(&serde_json::to_value(&*found).unwrap(), search);
    let create = r#"{"id":"vs_1","object":"vector_store","created_at":1700000000,"name":"docs","bytes":2048,"file_counts":{"in_progress":0,"completed":1,"failed":0,"cancelled":0,"total":1},"status":"completed","expires_after":{"anchor":"last_active_at","days":7},"expires_at":1700604800,"last_active_at":1700000000,"metadata":{"team":"core"}}"#;
    let Ok(VectorReply::Record(store)) = reply_of(ReplyKind::Create, 200, create).await else {
        panic!("the create reply decodes to a record");
    };
    let VectorStoreRecord::CreateResponse(created) = &*store else {
        panic!("a create reply is a create response record");
    };
    assert_eq!(created.id.as_deref(), Some("vs_1"));
    assert_eq!(created.name.as_deref(), Some("docs"));
    assert_eq!(created.status, Some(VectorStoreStatus::Completed));
    assert_json(&serde_json::to_value(&*store).unwrap(), create);
}

async fn file_reply_row() {
    let attached = r#"{"id":"file_1","object":"vector_store.file","created_at":1700000000,"usage_bytes":12,"vector_store_id":"vs_1","status":"completed","last_error":null,"chunking_strategy":{"type":"static","static":{"max_chunk_size_tokens":800,"chunk_overlap_tokens":400}},"attributes":{"team":"core"}}"#;
    let Ok(VectorReply::Json(kept)) = reply_of(ReplyKind::File, 200, attached).await else {
        panic!("the attached-file reply stays JSON");
    };
    assert_json(&kept, attached);
}

async fn reply_error_row() {
    let error = reply_of(ReplyKind::Create, 502, "{").await.unwrap_err();
    assert!(matches!(
        error,
        VectorStoreError::InvalidReply { status, .. } if status.as_u16() == 502
    ));
}

pub(crate) async fn check() {
    credential_rows().await;
    environment_rows().await;
    url_rows().await;
    endpoint_rows().await;
    search_request_row().await;
    create_request_rows().await;
    attached_file_request_rows().await;
    file_operation_rows().await;
    file_operation_error_row().await;
    reply_rows().await;
    file_reply_row().await;
    reply_error_row().await;
}
