#![cfg(test)]

use super::error::VectorStoreError;
use super::files::{self, Attributes, FileCreateParams, FileListParams, FileOperation};
use super::reply::{ReplyKind, decode_vector_reply, transform_response};
use super::store::{self, CreateParams, Query, SearchParams};
use super::{
    Headers, complete_url, file_endpoints, get_auth_credentials, store_endpoints,
    validate_environment,
};
use crate::credentials::LiteLlmGlobals;
use betterlite_config::sections::openai_credentials::Settings;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const BASE: &str = "https://h.example/v1/vector_stores";

fn parse(text: &str) -> Value {
    serde_json::from_str(text).expect("json")
}

fn key(value: &str) -> SecretString {
    SecretString::from(value)
}

fn settings() -> Settings {
    Settings {
        api_base: String::from("https://env.example/v1"),
        organization: None,
    }
}

fn plain(headers: &Headers) -> BTreeMap<&str, &str> {
    let exposed = headers.iter();
    exposed
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

fn caller(pairs: &[(&str, &str)]) -> Headers {
    let owned = pairs.iter();
    owned
        .map(|(name, value)| ((*name).to_owned(), key(value)))
        .collect()
}

fn globals(api_key: &str, openai_key: &str) -> LiteLlmGlobals {
    LiteLlmGlobals {
        api_base: None,
        api_key: Some(key(api_key)),
        openai_key: Some(key(openai_key)),
        organization: None,
    }
}

fn response(status: u16, body: &[u8]) -> http::Response<Vec<u8>> {
    let built = http::Response::builder()
        .status(status)
        .header("x-request-id", "req_1");
    built.body(body.to_vec()).expect("response")
}

fn store_create() {
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
    let request = store::create_request(&params, BASE);
    let sent = request.body["metadata"].as_object().expect("metadata");
    assert_eq!((request.url.as_str(), sent.len()), (BASE, 16));
    assert!(sent.contains_key("k15") && !sent.contains_key("k16") && !sent.contains_key("count"));
    let defaults = store::create_request(&CreateParams::default(), BASE).body;
    let keys = [
        "name",
        "file_ids",
        "expires_after",
        "chunking_strategy",
        "metadata",
    ];
    assert_eq!(defaults.as_object().map(Map::len), Some(5));
    assert!(keys.iter().all(|name| defaults[name].is_null()));
}

fn search_ranking_filter() {
    let query = Query::Many(vec![String::from("a"), String::from("b")]);
    let params = SearchParams {
        filters: Some(parse(r#"{"type": "eq"}"#)),
        max_num_results: Some(5),
        ranking_options: Some(parse(r#"{"ranker": "auto"}"#)),
        rewrite_query: Some(false),
    };
    let sent = store::search_request("vs_1", &query, &params, BASE).expect("search");
    let expected = parse(
        r#"{"query": ["a", "b"], "filters": {"type": "eq"}, "max_num_results": 5,
            "ranking_options": {"ranker": "auto"}, "rewrite_query": false}"#,
    );
    assert_eq!(
        (sent.url, sent.body),
        (format!("{BASE}/vs_1/search"), expected)
    );
    let one = Query::One(String::from("q"));
    let bare = store::search_request("vs_1", &one, &SearchParams::default(), BASE).expect("bare");
    let nulls = parse(
        r#"{"query": "q", "filters": null, "max_num_results": null,
            "ranking_options": null, "rewrite_query": null}"#,
    );
    assert_eq!(bare.body, nulls);
}

fn file_create_chunking() {
    let attributes = IndexMap::from([
        (String::from("team"), Value::from("a")),
        (String::from("rank"), Value::from(2_i64)),
    ]);
    let strategy = parse(
        r#"{"type": "static", "static": {"max_chunk_size_tokens": 800, "chunk_overlap_tokens": 400}}"#,
    );
    let params = FileCreateParams {
        file_id: Some(String::from("file_1")),
        attributes: Some(Attributes::Map(attributes)),
        chunking_strategy: Some(serde_json::from_value(strategy.clone()).expect("chunking")),
    };
    let request = files::create_request(&params, BASE);
    let mut expected = parse(r#"{"file_id": "file_1", "attributes": {"team": "a"}}"#);
    expected["chunking_strategy"] = strategy;
    assert_eq!((request.url.as_str(), request.body), (BASE, expected));
    let empty = files::create_request(&FileCreateParams::default(), BASE);
    assert_eq!(empty.body, parse("{}"));
}

fn file_list_pagination() {
    let params = FileListParams {
        after: Some(String::from("file_9")),
        limit: Some(10),
        before: None,
        filter: None,
        order: Some(String::from("desc")),
    };
    let request = files::list_request(&params, BASE);
    let expected = parse(r#"{"after": "file_9", "limit": 10, "order": "desc"}"#);
    assert_eq!((request.url.as_str(), request.body), (BASE, expected));
}

fn file_retrieve_content() {
    let retrieve = files::file_request(&FileOperation::Retrieve, "f/1", BASE).expect("retrieve");
    let content = files::file_request(&FileOperation::Content, "f/1", BASE).expect("content");
    assert_eq!(
        (retrieve.url, retrieve.body),
        (format!("{BASE}/f%2F1"), parse("{}"))
    );
    assert_eq!(
        (content.url, content.body),
        (format!("{BASE}/f%2F1/content"), parse("{}"))
    );
    let cast = transform_response(ReplyKind::File, &response(200, b"[1, \"x\"]")).expect("cast");
    assert_eq!(
        serde_json::to_value(cast).expect("json"),
        parse(r#"[1, "x"]"#)
    );
}

fn file_update_attributes() {
    let many = IndexMap::from([
        (String::from("a"), Value::from("1")),
        (String::from("b"), Value::from(true)),
    ]);
    let update = |attributes| {
        let operation = FileOperation::Update(files::FileUpdateParams { attributes });
        files::file_request(&operation, "f1", BASE).expect("update")
    };
    let kept = update(Attributes::Map(many)).body;
    assert_eq!(kept, parse(r#"{"attributes": {"a": "1"}}"#));
    let other = update(Attributes::Other(Value::from(7_i64))).body;
    assert_eq!(other, parse(r#"{"attributes": 7}"#));
    let null = update(Attributes::Other(Value::Null));
    assert_eq!(
        (null.url, null.body),
        (format!("{BASE}/f1"), parse(r#"{"attributes": null}"#))
    );
}

fn file_delete() {
    let request = files::file_request(&FileOperation::Delete, "f 1", BASE).expect("delete");
    assert_eq!(
        (request.url, request.body),
        (format!("{BASE}/f%201"), parse("{}"))
    );
}

fn assistants_beta_default() {
    let none = LiteLlmGlobals::default;
    let added = validate_environment(Headers::new(), Some(key("k")), none(), None);
    assert_eq!(plain(&added)["OpenAI-Beta"], "assistants=v2");
    let empty = caller(&[("OpenAI-Beta", "")]);
    let kept = validate_environment(empty, Some(key("k")), none(), None);
    assert_eq!(plain(&kept)["OpenAI-Beta"], "");
    let lowercase = caller(&[("openai-beta", "x")]);
    let both = validate_environment(lowercase, Some(key("k")), none(), None);
    let shown = plain(&both);
    assert_eq!(
        (shown["OpenAI-Beta"], shown["openai-beta"]),
        ("assistants=v2", "x")
    );
}

fn credential_precedence() {
    let pick = |explicit: Option<SecretString>, global, env| {
        let stale = caller(&[
            ("Authorization", "Bearer old"),
            ("Content-Type", "text/plain"),
        ]);
        validate_environment(stale, explicit, global, env)
    };
    let first = pick(Some(key("p")), globals("g", "o"), Some(key("e")));
    let shown = plain(&first);
    assert_eq!(
        (shown["Authorization"], shown["Content-Type"]),
        ("Bearer p", "application/json")
    );
    let order = [
        (Some(key("")), globals("g", "o"), Some(key("e")), "Bearer g"),
        (None, globals("", "o"), Some(key("e")), "Bearer o"),
        (None, globals("", ""), Some(key("e")), "Bearer e"),
        (None, LiteLlmGlobals::default(), None, "Bearer None"),
    ];
    for (explicit, global, env, expected) in order {
        let headers = pick(explicit, global, env);
        assert_eq!(plain(&headers)["Authorization"], expected);
    }
}

fn auth_key_required() {
    assert!(matches!(
        get_auth_credentials(None),
        Err(VectorStoreError::MissingApiKey)
    ));
    let empty = get_auth_credentials(Some(&key(""))).expect("empty key");
    assert_eq!(
        plain(&empty),
        BTreeMap::from([("Authorization", "Bearer ")])
    );
}

fn base_url_precedence() {
    let url = |explicit, global, config: &Settings, id| {
        complete_url(explicit, global, config, id).expect("url")
    };
    let both = url(
        Some("https://a.example/"),
        Some("https://g.example"),
        &settings(),
        None,
    );
    assert_eq!(both, "https://a.example/vector_stores");
    let global = url(Some(""), Some("https://g.example///"), &settings(), None);
    assert_eq!(global, "https://g.example/vector_stores");
    let env = url(None, Some(""), &settings(), None);
    assert_eq!(env, "https://env.example/v1/vector_stores");
    let default = url(None, None, &Settings::default(), Some("vs_1"));
    assert_eq!(
        default,
        "https://api.openai.com/v1/vector_stores/vs_1/files"
    );
}

fn encoded_ids_query() {
    let base = "https://h.example/v1/vector_stores?api-version=1";
    let query = Query::One(String::from("q"));
    let search = store::search_request("a/b c", &query, &SearchParams::default(), base);
    assert_eq!(
        search.expect("search").url,
        "https://h.example/v1/vector_stores/a%2Fb%20c/search?api-version=1"
    );
    let files = complete_url(Some("https://h.example"), None, &settings(), Some("a/b"));
    assert_eq!(
        files.expect("url"),
        "https://h.example/vector_stores/a%2Fb/files"
    );
    for bad in ["", ".", ".."] {
        let file = files::file_request(&FileOperation::Retrieve, bad, BASE);
        assert!(matches!(file, Err(VectorStoreError::Segment(_))));
        let store = complete_url(None, None, &settings(), Some(bad));
        assert!(matches!(store, Err(VectorStoreError::Segment(_))));
    }
}

fn endpoint_permissions() {
    let pairs = |list: Option<Vec<(String, String)>>| {
        let listed = list.expect("endpoints").into_iter();
        listed
            .map(|(method, path)| format!("{method} {path}"))
            .collect::<Vec<_>>()
    };
    let store = store_endpoints();
    assert_eq!(
        pairs(store.read),
        ["GET /vector_stores/{index_name}/search"]
    );
    assert_eq!(pairs(store.write), ["POST /vector_stores"]);
    let file = file_endpoints();
    let base = "/vector_stores/{vector_store_id}/files";
    let item = format!("{base}/{{file_id}}");
    let read = [
        format!("GET {base}"),
        format!("GET {item}"),
        format!("GET {item}/content"),
    ];
    let write = [
        format!("POST {base}"),
        format!("POST {item}"),
        format!("DELETE {item}"),
    ];
    assert_eq!(pairs(file.read), read);
    assert_eq!(pairs(file.write), write);
}

fn invalid_reply_status_headers() {
    for kind in [ReplyKind::Search, ReplyKind::Create, ReplyKind::File] {
        let error = transform_response(kind, &response(502, b"{")).expect_err("malformed");
        assert!(
            matches!(&error, VectorStoreError::InvalidReply { status, headers, .. }
                if status.as_u16() == 502 && headers["x-request-id"] == "req_1"),
            "{kind:?}"
        );
    }
    transform_response(ReplyKind::Search, &response(200, b"[1]")).expect_err("array");
    transform_response(ReplyKind::File, &response(200, b"")).expect_err("empty");
}

#[rstest]
#[case::store_create(store_create)]
#[case::search_ranking_filter(search_ranking_filter)]
#[case::file_create_chunking(file_create_chunking)]
#[case::file_list_pagination(file_list_pagination)]
#[case::file_retrieve_content(file_retrieve_content)]
#[case::file_update_attributes(file_update_attributes)]
#[case::file_delete(file_delete)]
#[case::assistants_beta_default(assistants_beta_default)]
#[case::credential_precedence(credential_precedence)]
#[case::auth_key_required(auth_key_required)]
#[case::base_url_precedence(base_url_precedence)]
#[case::encoded_ids_query(encoded_ids_query)]
#[case::endpoint_permissions(endpoint_permissions)]
#[case::invalid_reply_status_headers(invalid_reply_status_headers)]
fn vector_stores_cases(#[case] scenario: fn()) {
    scenario();
}

#[test]
fn openai_vector_stores_credential_identity_isolation() {
    let none = LiteLlmGlobals::default;
    let team = caller(&[("X-Team", "a")]);
    let first = validate_environment(team, Some(key("key-a")), none(), None);
    let second = validate_environment(Headers::new(), Some(key("key-b")), none(), None);
    assert_eq!(plain(&first)["Authorization"], "Bearer key-a");
    assert_eq!(plain(&second)["Authorization"], "Bearer key-b");
    assert!(!second.contains_key("X-Team"));
    let env = Some(key("key-a"));
    let third = validate_environment(Headers::new(), None, globals("key-c", ""), env);
    assert_eq!(plain(&third)["Authorization"], "Bearer key-c");
}

#[test]
fn openai_vector_stores_secret_safe_failure() {
    let none = LiteLlmGlobals::default;
    let headers = validate_environment(Headers::new(), Some(key("sk-fixture")), none(), None);
    let missing = get_auth_credentials(None).expect_err("missing key");
    let invalid = transform_response(ReplyKind::Search, &response(401, b"{")).expect_err("bad");
    let segment = files::file_request(&FileOperation::Retrieve, "..", BASE).expect_err("dot");
    let shown =
        format!("{headers:?} {missing:?} {missing} {invalid:?} {invalid} {segment:?} {segment}");
    assert!(!shown.contains("sk-fixture"));
}

#[test]
fn replays_the_openai_vector_stores_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_vector_stores");
    let paths: Vec<PathBuf> = fs::read_dir(&corpus)
        .expect("corpus folder")
        .map(|entry| entry.expect("corpus entry").path())
        .collect();
    assert!(!paths.is_empty());
    let wrong: Vec<&str> = paths
        .iter()
        .filter(|path| {
            let bytes = fs::read(path).expect("corpus file");
            let (&tag, body) = bytes.split_first().expect("kind byte");
            let kind = ReplyKind::from_tag(tag).expect("kind tag");
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("name");
            decode_vector_reply(kind, body).is_ok() != name.starts_with("ok-")
        })
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("?")
        })
        .collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}
