use std::collections::BTreeMap;

use betterlite_config::sections::openai_credentials::Settings;
use betterlite_openai::credentials::LiteLlmGlobals;
use betterlite_openai::vector_stores::error::VectorStoreError;
use betterlite_openai::vector_stores::files::{
    Attributes, FileCreateParams, FileListParams, FileOperation, FileUpdateParams,
};
use betterlite_openai::vector_stores::reply::VectorReply;
use betterlite_openai::vector_stores::store::{CreateParams, Query, SearchParams};
use betterlite_openai::vector_stores::{Headers, Request};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};

use super::fixture::object;
use crate::openai_resource_dispatch::vector_stores::{
    Call, Dispatch, EndpointKind, Outcome, ReplyCall, RequestCall,
};
use crate::openai_resource_dispatch::{
    OpenAiResourceError, OpenAiResourceOperation, OpenAiResourceResult, dispatch,
};

const API_BASE: &str = "https://h.example/v1/vector_stores";

fn key(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings() -> Settings {
    Settings {
        api_base: String::from("https://env.example/v1"),
        organization: None,
    }
}

fn plain(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

fn reply(status: u16, body: &[u8]) -> http::Response<Vec<u8>> {
    http::Response::builder()
        .status(status)
        .body(body.to_vec())
        .unwrap()
}

async fn run(
    settings: &Settings,
    globals: LiteLlmGlobals,
    env_api_key: Option<SecretString>,
    call: Call<'_>,
) -> Result<OpenAiResourceResult, OpenAiResourceError> {
    let operation = OpenAiResourceOperation::VectorStores(Box::new(Dispatch {
        settings,
        globals,
        env_api_key,
        call,
    }));
    dispatch(operation).await
}

fn globals(api_key: Option<&str>) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: None,
        api_key: api_key.map(key),
        openai_key: None,
        organization: None,
    }
}

async fn credential_rows() {
    let settings = settings();
    let given = key("sk-vs");
    let headers = run(&settings, globals(None), None, Call::Auth(Some(&given))).await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Headers(headers))) = headers else {
        panic!("the auth call returns headers");
    };
    assert_eq!(
        plain(&headers),
        BTreeMap::from([("Authorization", "Bearer sk-vs")])
    );
    let missing = run(&settings, globals(None), None, Call::Auth(None)).await;
    let Err(error) = missing else {
        panic!("a missing vector store key is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::VectorStores(VectorStoreError::MissingApiKey)
    ));
}

async fn environment_rows() {
    let settings = settings();
    let added = run(
        &settings,
        globals(None),
        None,
        Call::Environment {
            headers: Headers::new(),
            explicit_key: Some(key("p")),
        },
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Headers(headers))) = added else {
        panic!("the environment call returns headers");
    };
    assert_eq!(
        plain(&headers),
        BTreeMap::from([
            ("Authorization", "Bearer p"),
            ("Content-Type", "application/json"),
            ("OpenAI-Beta", "assistants=v2"),
        ])
    );
    let kept = run(
        &settings,
        globals(Some("g")),
        None,
        Call::Environment {
            headers: Headers::from([(String::from("OpenAI-Beta"), key(""))]),
            explicit_key: None,
        },
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Headers(headers))) = kept else {
        panic!("the environment call returns headers");
    };
    assert_eq!(plain(&headers)["OpenAI-Beta"], "");
    assert_eq!(plain(&headers)["Authorization"], "Bearer g");
}

async fn url_rows() {
    let settings = settings();
    let absolute = run(
        &settings,
        globals(None),
        None,
        Call::Url {
            api_base: Some("https://a.example/"),
            litellm_api_base: None,
            vector_store_id: None,
        },
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Url(url))) = absolute else {
        panic!("the url call returns its url");
    };
    assert_eq!(url, "https://a.example/vector_stores");
    let files = run(
        &settings,
        globals(None),
        None,
        Call::Url {
            api_base: None,
            litellm_api_base: Some("https://g.example///"),
            vector_store_id: Some("vs_1"),
        },
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Url(url))) = files else {
        panic!("the url call returns its url");
    };
    assert_eq!(url, "https://g.example/vector_stores/vs_1/files");
    let encoded = run(
        &settings,
        globals(None),
        None,
        Call::Url {
            api_base: None,
            litellm_api_base: None,
            vector_store_id: Some("a/b"),
        },
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Url(url))) = encoded else {
        panic!("the url call returns its url");
    };
    assert_eq!(url, "https://env.example/v1/vector_stores/a%2Fb/files");
    let bad = run(
        &settings,
        globals(None),
        None,
        Call::Url {
            api_base: None,
            litellm_api_base: None,
            vector_store_id: Some(".."),
        },
    )
    .await;
    let Err(error) = bad else {
        panic!("a traversal id is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::VectorStores(VectorStoreError::Segment(_))
    ));
}

async fn endpoint_rows() {
    let settings = settings();
    let store = run(
        &settings,
        globals(None),
        None,
        Call::Endpoints(EndpointKind::Store),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Endpoints(endpoints))) = store else {
        panic!("the store endpoints call returns endpoints");
    };
    assert_eq!(
        endpoints.read,
        Some(vec![(
            String::from("GET"),
            String::from("/vector_stores/{index_name}/search")
        )])
    );
    assert_eq!(
        endpoints.write,
        Some(vec![(String::from("POST"), String::from("/vector_stores"))])
    );
    let files = run(
        &settings,
        globals(None),
        None,
        Call::Endpoints(EndpointKind::File),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Endpoints(endpoints))) = files else {
        panic!("the file endpoints call returns endpoints");
    };
    assert_eq!(endpoints.read.as_ref().map(Vec::len), Some(3));
    assert_eq!(endpoints.write.as_ref().map(Vec::len), Some(3));
}

async fn search_request_rows() {
    let settings = settings();
    let query = Query::Many(vec![String::from("a"), String::from("b")]);
    let params = SearchParams {
        filters: Some(object(r#"{"type":"eq"}"#)),
        max_num_results: Some(5),
        ranking_options: Some(object(r#"{"ranker":"auto"}"#)),
        rewrite_query: Some(false),
    };
    let base = "https://h.example/v1/vector_stores?api-version=1";
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::Search {
            vector_store_id: "a/b c",
            query: &query,
            params: &params,
            api_base: base,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = result else {
        panic!("the search call returns a request");
    };
    let Request { url, body } = request;
    assert_eq!(
        url,
        "https://h.example/v1/vector_stores/a%2Fb%20c/search?api-version=1"
    );
    assert_eq!(
        body,
        Value::Object(object(
            r#"{"query":["a","b"],"filters":{"type":"eq"},"max_num_results":5,"ranking_options":{"ranker":"auto"},"rewrite_query":false}"#
        ))
    );
}

async fn search_response_row() {
    let settings = settings();
    let body = br#"{"object":"vector_store.search_results.page","search_query":"q","data":[]}"#;
    let response = reply(200, body);
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Reply(ReplyCall::Search(&response)),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Reply(reply))) = result else {
        panic!("the search reply returns a decoded reply");
    };
    let VectorReply::Record(record) = reply.as_ref() else {
        panic!("a search reply decodes as a search record");
    };
    assert_eq!(
        serde_json::to_value(record).unwrap()["object"],
        Value::from("vector_store.search_results.page")
    );
}

async fn create_request_rows() {
    let settings = settings();
    let mut metadata: IndexMap<String, Value> = (0..20_u8)
        .map(|index| (format!("k{index}"), Value::from("v")))
        .collect();
    metadata.insert(String::from("hidden_params"), Value::from("x"));
    metadata.shift_insert(1, String::from("count"), Value::from(3_i64));
    let params = CreateParams {
        name: Some(String::from("docs")),
        file_ids: Some(vec![String::from("f1")]),
        expires_after: None,
        chunking_strategy: None,
        metadata: Some(metadata),
    };
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::Create {
            params: &params,
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = result else {
        panic!("the store create call returns a request");
    };
    assert_eq!(request.url, API_BASE);
    let sent = request.body["metadata"].as_object().unwrap();
    assert_eq!(sent.len(), 16);
    assert!(sent.contains_key("k15") && !sent.contains_key("k16") && !sent.contains_key("count"));
    let defaults = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::Create {
            params: &CreateParams::default(),
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = defaults else {
        panic!("the store create call returns a request");
    };
    assert_eq!(
        request.body,
        Value::Object(object(
            r#"{"name":null,"file_ids":null,"expires_after":null,"chunking_strategy":null,"metadata":null}"#
        ))
    );
}

async fn create_response_row() {
    let settings = settings();
    let body = br#"{"id":"vs_a","object":"vector_store","created_at":1,"name":null,"bytes":2,"file_counts":{"in_progress":1,"completed":2,"failed":3,"cancelled":4,"total":10},"status":"expired","expires_after":{"anchor":"last_active_at","days":3},"expires_at":null,"last_active_at":1,"metadata":{"tag":"x"}}"#;
    let response = reply(200, body);
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Reply(ReplyCall::Create(&response)),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Reply(reply))) = result else {
        panic!("the create reply returns a decoded reply");
    };
    let VectorReply::Record(record) = reply.as_ref() else {
        panic!("a create reply decodes as a create record");
    };
    assert_eq!(
        serde_json::to_value(record).unwrap()["id"],
        Value::from("vs_a")
    );
}

async fn file_create_row() {
    let settings = settings();
    let attributes = IndexMap::from([
        (String::from("team"), Value::from("a")),
        (String::from("rank"), Value::from(2_i64)),
    ]);
    let params = FileCreateParams {
        file_id: Some(String::from("file_1")),
        attributes: Some(Attributes::Map(attributes)),
        chunking_strategy: None,
    };
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::FileCreate {
            params: &params,
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = result else {
        panic!("the file create call returns a request");
    };
    assert_eq!(request.url, API_BASE);
    assert_eq!(
        request.body,
        Value::Object(object(r#"{"file_id":"file_1","attributes":{"team":"a"}}"#))
    );
}

async fn file_list_row() {
    let settings = settings();
    let params = FileListParams {
        after: Some(String::from("file_9")),
        before: None,
        filter: None,
        limit: Some(10),
        order: Some(String::from("desc")),
    };
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::FileList {
            params: &params,
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = result else {
        panic!("the file list call returns a request");
    };
    assert_eq!(request.url, API_BASE);
    assert_eq!(
        request.body,
        Value::Object(object(r#"{"after":"file_9","limit":10,"order":"desc"}"#))
    );
}

async fn file_request_rows() {
    let settings = settings();
    let cases = [
        (FileOperation::Retrieve, "/f%2F1", object("{}")),
        (FileOperation::Content, "/f%2F1/content", object("{}")),
        (FileOperation::Delete, "/f%2F1", object("{}")),
    ];
    for (operation, suffix, body) in cases {
        let result = run(
            &settings,
            globals(None),
            None,
            Call::Request(RequestCall::File {
                operation: &operation,
                file_id: "f/1",
                api_base: API_BASE,
            }),
        )
        .await;
        let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = result else {
            panic!("the file call returns a request");
        };
        assert_eq!(request.url, format!("{API_BASE}{suffix}"));
        assert_eq!(request.body, Value::Object(body));
    }
    let update = FileOperation::Update(FileUpdateParams {
        attributes: Attributes::Other(Value::from(7_i64)),
    });
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::File {
            operation: &update,
            file_id: "f1",
            api_base: API_BASE,
        }),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Request(request))) = result else {
        panic!("the file update call returns a request");
    };
    assert_eq!(request.body, Value::Object(object(r#"{"attributes":7}"#)));
    let bad = run(
        &settings,
        globals(None),
        None,
        Call::Request(RequestCall::File {
            operation: &FileOperation::Retrieve,
            file_id: "..",
            api_base: API_BASE,
        }),
    )
    .await;
    let Err(error) = bad else {
        panic!("a traversal file id is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::VectorStores(VectorStoreError::Segment(_))
    ));
}

async fn file_response_row() {
    let settings = settings();
    let response = reply(200, b"[1, \"x\"]");
    let result = run(
        &settings,
        globals(None),
        None,
        Call::Reply(ReplyCall::File(&response)),
    )
    .await;
    let Ok(OpenAiResourceResult::VectorStores(Outcome::Reply(reply))) = result else {
        panic!("the file reply returns a decoded reply");
    };
    assert_eq!(
        serde_json::to_value(reply.as_ref()).unwrap(),
        Value::Array(vec![Value::from(1), Value::from("x")])
    );
    let broken = reply(500, b"not json");
    let failed = run(
        &settings,
        globals(None),
        None,
        Call::Reply(ReplyCall::File(&broken)),
    )
    .await;
    let Err(error) = failed else {
        panic!("a malformed file reply is a leaf error");
    };
    assert!(matches!(
        error,
        OpenAiResourceError::VectorStores(VectorStoreError::InvalidReply { .. })
    ));
}

pub(crate) async fn check() {
    credential_rows().await;
    environment_rows().await;
    url_rows().await;
    endpoint_rows().await;
    search_request_rows().await;
    search_response_row().await;
    create_request_rows().await;
    create_response_row().await;
    file_create_row().await;
    file_list_row().await;
    file_request_rows().await;
    file_response_row().await;
}
