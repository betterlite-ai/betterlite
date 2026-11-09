#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;

use super::error::PgVectorError;
use super::{PgVectorEnv, PgVectorParams, get_complete_url, search_request, validate_environment};
use crate::vector_stores::Headers;
use crate::vector_stores::store::{Query, SearchParams, search_request as openai_search_request};

const BASE: &str = "https://pg.example.test/v1/vector_stores";

fn key(value: &str) -> SecretString {
    SecretString::from(value)
}

fn params(api_key: Option<&str>) -> PgVectorParams {
    PgVectorParams {
        api_key: api_key.map(key),
    }
}

fn env(api_key: Option<&str>, api_base: Option<&str>) -> PgVectorEnv {
    PgVectorEnv {
        api_key: api_key.map(key),
        api_base: api_base.map(str::to_owned),
    }
}

fn plain(headers: &Headers) -> BTreeMap<&str, &str> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str(), value.expose_secret()))
        .collect()
}

fn json_value(text: &str) -> Value {
    serde_json::from_str(text).expect("json")
}

fn search_params(text: &str) -> SearchParams {
    serde_json::from_str(text).expect("search params")
}

fn headers() {
    let given = BTreeMap::from([("X-Trace".to_owned(), key("t"))]);
    let sent = validate_environment(
        given,
        Some(&params(Some("explicit-key"))),
        &env(Some("env-key"), None),
    )
    .expect("explicit key");
    let shown = plain(&sent);
    assert_eq!(shown["Authorization"], "Bearer explicit-key");
    assert_eq!(shown["Content-Type"], "application/json");
    assert_eq!(shown["X-Trace"], "t");
    let from_env = validate_environment(
        Headers::new(),
        Some(&params(None)),
        &env(Some("env-key"), None),
    )
    .expect("env key");
    assert_eq!(plain(&from_env)["Authorization"], "Bearer env-key");
    let empty_explicit = validate_environment(
        Headers::new(),
        Some(&params(Some(""))),
        &env(Some("env-key"), None),
    )
    .expect("empty explicit key falls back");
    assert_eq!(plain(&empty_explicit)["Authorization"], "Bearer env-key");
    let no_params = validate_environment(Headers::new(), None, &env(Some("env-key"), None))
        .expect("absent params use the env key");
    assert_eq!(plain(&no_params)["Authorization"], "Bearer env-key");
    let missing = validate_environment(Headers::new(), None, &PgVectorEnv::default())
        .expect_err("missing key");
    assert!(matches!(missing, PgVectorError::MissingApiKey));
    assert_eq!(
        missing.to_string(),
        "PG Vector API key is required. Set PG_VECTOR_API_KEY environment variable or pass api_key in litellm_params."
    );
}

fn base_url() {
    let explicit = get_complete_url(Some("https://pg.example.test///"), &env(None, None));
    assert_eq!(explicit.expect("explicit"), BASE);
    let from_env = get_complete_url(None, &env(None, Some("https://env.example.test//")));
    assert_eq!(
        from_env.expect("env"),
        "https://env.example.test/v1/vector_stores"
    );
    let empty_explicit = get_complete_url(Some(""), &env(None, Some("https://env.example.test/")));
    assert_eq!(
        empty_explicit.expect("empty explicit"),
        "https://env.example.test/v1/vector_stores"
    );
    let missing = get_complete_url(None, &PgVectorEnv::default()).expect_err("missing base");
    assert!(matches!(missing, PgVectorError::MissingApiBase));
    assert_eq!(
        missing.to_string(),
        "PG Vector API base URL is required. Set PG_VECTOR_API_BASE environment variable or pass api_base in litellm_params."
    );
}

fn search_url() {
    let params = search_params(
        r#"{"filters":{"type":"eq"},"max_num_results":5,
            "ranking_options":{"ranker":"auto"},"rewrite_query":false}"#,
    );
    let query = Query::Many(vec!["one".to_owned(), "two".to_owned()]);
    let sent = search_request("vs/a b", &query, &params, BASE).expect("encoded id");
    assert_eq!(
        sent.url,
        "https://pg.example.test/v1/vector_stores/vs%2Fa%20b/search"
    );
    let expected = json_value(
        r#"{"query":["one","two"],"filters":{"type":"eq"},"max_num_results":5,
            "ranking_options":{"ranker":"auto"},"rewrite_query":false}"#,
    );
    assert_eq!(sent.body, expected);
    let openai = openai_search_request("vs/a b", &query, &params, BASE).expect("openai body");
    assert_eq!(sent.body, openai.body);
}

fn search_body_defaults() {
    let query = Query::One("hello".to_owned());
    let sent = search_request("vs_1", &query, &SearchParams::default(), BASE).expect("search");
    assert_eq!(sent.url, format!("{BASE}/vs_1/search"));
    assert_eq!(
        sent.body,
        json_value(
            r#"{"query":"hello","filters":null,"max_num_results":null,
                "ranking_options":null,"rewrite_query":null}"#
        )
    );
}

fn search_url_is_not_the_openai_one() {
    let base = "https://pg.example.test/v1/vector_stores?api-version=1";
    let query = Query::One("hello".to_owned());
    let sent = search_request("vs_1", &query, &SearchParams::default(), base).expect("search");
    assert_eq!(
        sent.url,
        "https://pg.example.test/v1/vector_stores?api-version=1/vs_1/search"
    );
    let openai =
        openai_search_request("vs_1", &query, &SearchParams::default(), base).expect("openai url");
    assert_eq!(
        openai.url,
        "https://pg.example.test/v1/vector_stores/vs_1/search?api-version=1"
    );
    assert_eq!(sent.body, openai.body);
}

fn search_id_errors() {
    let query = Query::One("hello".to_owned());
    for bad in ["", ".", ".."] {
        let error = search_request(bad, &query, &SearchParams::default(), BASE)
            .expect_err("invalid segment");
        assert!(matches!(error, PgVectorError::Segment(_)), "{bad}");
    }
}

#[rstest]
#[case::headers(headers)]
#[case::base_url(base_url)]
#[case::search_url(search_url)]
#[case::search_body_defaults(search_body_defaults)]
#[case::search_url_is_not_the_openai_one(search_url_is_not_the_openai_one)]
#[case::search_id_errors(search_id_errors)]
fn pg_vector_store_cases(#[case] scenario: fn()) {
    scenario();
}
