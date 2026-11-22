#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::SecretString;

use super::error::PerplexitySearchError;
use super::reply::{MAX_REPLY_BYTES, decode_search_reply};
use super::{PerplexitySearch, PerplexitySearchRequest};
use crate::openai_base_config::{Headers, Object};
use crate::perplexity_embedding::reply::transform_embedding_response;
use crate::search::credentials::CredentialError;
use crate::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider as _, SearchQuery,
};

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

fn request(
    search: &PerplexitySearch<'_>,
    query: &SearchQuery,
    optional_params: &Object,
) -> Result<PerplexitySearchRequest, PerplexitySearchError> {
    search.transform_search_request(&RequestInput {
        query,
        optional_params,
        api_key: None,
        api_base: None,
        headers: &Headers::new(),
    })
}

#[test]
fn the_provider_is_perplexity_over_post() {
    let variables = environment(&[]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    assert_eq!(PerplexitySearch::ui_friendly_name(), "Perplexity");
    assert_eq!(search.http_method(), HttpMethod::Post);
}

#[rstest]
#[case::explicit_key(Some("sk-call"), &[("PERPLEXITYAI_API_KEY", "sk-env")], "Bearer sk-call")]
#[case::server_key(None, &[("PERPLEXITYAI_API_KEY", "sk-env")], "Bearer sk-env")]
#[case::empty_explicit_key(Some(""), &[("PERPLEXITYAI_API_KEY", "sk-env")], "Bearer sk-env")]
fn a_resolved_key_sets_the_bearer_and_json_type(
    #[case] api_key: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] authorization: &str,
) {
    let variables = environment(pairs);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let key = api_key.map(secret);
    let caller = Headers::from([(String::from("X-Trace"), String::from("t1"))]);
    let headers = search
        .validated_headers(caller.clone(), key.as_ref(), None)
        .unwrap();
    assert_eq!(
        headers,
        Headers::from([
            (String::from("X-Trace"), String::from("t1")),
            (String::from("Authorization"), authorization.to_owned()),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
        ])
    );
    assert_eq!(
        search.validate_environment(caller, key.as_ref(), None),
        headers
    );
}

#[rstest]
#[case::no_variable(&[])]
#[case::only_the_other_name(&[("PERPLEXITY_API_KEY", "sk-other")])]
#[case::empty_variable(&[("PERPLEXITYAI_API_KEY", "")])]
fn a_missing_key_names_perplexityai_api_key(#[case] pairs: &[(&str, &str)]) {
    let variables = environment(pairs);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let error = search
        .validated_headers(Headers::new(), None, None)
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "PERPLEXITYAI_API_KEY is not set. Set `PERPLEXITYAI_API_KEY` environment variable."
    );
    let body = PerplexitySearchRequest {
        query: SearchQuery::Single(String::from("q")),
        max_results: None,
        search_domain_filter: None,
        max_tokens_per_page: None,
        country: None,
    };
    let outbound = OutboundSearch {
        api_base: "https://api.perplexity.ai",
        api_key: None,
        optional_params: &Object::new(),
        body: &body,
    };
    assert!(matches!(
        search.sign_request(Headers::new(), &outbound),
        Err(PerplexitySearchError::MissingApiKey)
    ));
    assert_eq!(
        search.validate_environment(Headers::new(), None, None),
        Headers::new()
    );
}

#[test]
fn a_server_key_is_refused_for_a_foreign_base_and_kept_for_a_trusted_one() {
    let variables = environment(&[("PERPLEXITYAI_API_KEY", "sk-env")]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let refused = search
        .validated_headers(Headers::new(), None, Some("https://evil.example"))
        .unwrap_err();
    assert!(matches!(
        refused,
        PerplexitySearchError::Credential(CredentialError::UntrustedBase { .. })
    ));
    let trusted = search
        .validated_headers(Headers::new(), None, Some("https://api.perplexity.ai/v2"))
        .unwrap();
    assert_eq!(
        trusted.get("Authorization").map(String::as_str),
        Some("Bearer sk-env")
    );
}

#[test]
fn sign_request_validates_with_the_outbound_key() {
    let variables = environment(&[]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let key = secret("sk-call");
    let body = PerplexitySearchRequest {
        query: SearchQuery::Single(String::from("q")),
        max_results: None,
        search_domain_filter: None,
        max_tokens_per_page: None,
        country: None,
    };
    let outbound = OutboundSearch {
        api_base: "https://mock.example",
        api_key: Some(&key),
        optional_params: &Object::new(),
        body: &body,
    };
    let (headers, signed) = search.sign_request(Headers::new(), &outbound).unwrap();
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-call")
    );
    assert!(signed.is_none());
}

#[rstest]
#[case::explicit_base(Some("https://mock.example"), &[("PERPLEXITY_API_BASE", "https://env.example")], "https://mock.example/search")]
#[case::already_search(Some("https://mock.example/search"), &[], "https://mock.example/search")]
#[case::empty_base(Some(""), &[("PERPLEXITY_API_BASE", "https://env.example")], "https://env.example/search")]
#[case::variable_base(None, &[("PERPLEXITY_API_BASE", "https://env.example/search")], "https://env.example/search")]
#[case::empty_variable(None, &[("PERPLEXITY_API_BASE", "")], "https://api.perplexity.ai/search")]
#[case::default_base(None, &[], "https://api.perplexity.ai/search")]
fn the_url_appends_search_unless_present(
    #[case] api_base: Option<&str>,
    #[case] pairs: &[(&str, &str)],
    #[case] url: &str,
) {
    let variables = environment(pairs);
    let search = PerplexitySearch {
        environment: &variables,
    };
    assert_eq!(
        search
            .complete_url(api_base, &Object::new(), None, None)
            .unwrap(),
        url
    );
}

#[test]
fn the_request_keeps_the_four_options_and_drops_the_rest() {
    let variables = environment(&[]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let query = SearchQuery::Multiple(vec![String::from("a"), String::from("b")]);
    let optional = params(
        r#"{"max_results": 5, "search_domain_filter": ["x.example"], "max_tokens_per_page": 512, "country": "US", "search_recency_filter": "week"}"#,
    );
    let body = request(&search, &query, &optional).unwrap();
    assert_eq!(
        serde_json::to_string(&body).unwrap(),
        r#"{"query":["a","b"],"max_results":5,"search_domain_filter":["x.example"],"max_tokens_per_page":512,"country":"US"}"#
    );
}

#[test]
fn null_options_are_left_out() {
    let variables = environment(&[]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let query = SearchQuery::Single(String::from("q"));
    let optional = params(r#"{"max_results": null, "country": null}"#);
    let body = request(&search, &query, &optional).unwrap();
    assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"query":"q"}"#);
}

#[rstest]
#[case::max_results(r#"{"max_results": "five"}"#, "max_results")]
#[case::domains(r#"{"search_domain_filter": "x.example"}"#, "search_domain_filter")]
#[case::tokens(r#"{"max_tokens_per_page": [1]}"#, "max_tokens_per_page")]
#[case::country(r#"{"country": 7}"#, "country")]
fn an_option_of_the_wrong_type_names_it(#[case] text: &str, #[case] name: &str) {
    let variables = environment(&[]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let query = SearchQuery::Single(String::from("q"));
    let error = request(&search, &query, &params(text)).unwrap_err();
    assert!(matches!(
        error,
        PerplexitySearchError::OptionType { field, .. } if field == name
    ));
}

#[test]
fn results_keep_five_fields_with_their_defaults() {
    let reply = decode_search_reply(
        br#"{"id":"s1","results":[{"title":"T","url":"https://a.example","snippet":"S","date":"2025-01-01","last_updated":"2025-02-01","score":0.9},{"title":null},{}]}"#,
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&reply).unwrap(),
        r#"{"results":[{"title":"T","url":"https://a.example","snippet":"S","date":"2025-01-01","last_updated":"2025-02-01"},{"title":"","url":"","snippet":"","date":null,"last_updated":null},{"title":"","url":"","snippet":"","date":null,"last_updated":null}],"object":"search"}"#
    );
}

#[rstest]
#[case::no_results("{}")]
#[case::null_results(r#"{"results":null}"#)]
fn missing_results_give_an_empty_search(#[case] body: &str) {
    let reply = decode_search_reply(body.as_bytes()).unwrap();
    assert!(reply.results.is_empty());
    assert_eq!(reply.object.0, "search");
}

#[rstest]
#[case::not_json("upstream down")]
#[case::array("[]")]
#[case::result_not_object(r#"{"results":[1]}"#)]
#[case::date_not_text(r#"{"results":[{"date":5}]}"#)]
fn a_reply_of_the_wrong_shape_is_refused(#[case] body: &str) {
    assert!(matches!(
        decode_search_reply(body.as_bytes()),
        Err(PerplexitySearchError::Reply { .. })
    ));
}

#[test]
fn a_reply_over_the_limit_is_refused_and_one_at_the_limit_is_read() {
    let over = vec![b' '; MAX_REPLY_BYTES.checked_add(1).unwrap()];
    assert!(matches!(
        decode_search_reply(&over),
        Err(PerplexitySearchError::ReplyTooLarge { .. })
    ));
    let at_limit = vec![b' '; MAX_REPLY_BYTES];
    assert!(matches!(
        decode_search_reply(&at_limit),
        Err(PerplexitySearchError::Reply { .. })
    ));
}

#[test]
fn the_search_response_hook_decodes_the_reply_text() {
    let variables = environment(&[]);
    let search = PerplexitySearch {
        environment: &variables,
    };
    let headers = Headers::new();
    let reply = search
        .transform_search_response(&ProviderResponse {
            status_code: 200,
            headers: &headers,
            text: r#"{"results":[{"title":"T"}]}"#,
        })
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    let error = search.http_error(&ProviderResponse {
        status_code: 401,
        headers: &headers,
        text: "denied",
    });
    assert_eq!(error.status_code, 401);
    assert_eq!(error.message, "denied");
}

#[rstest]
fn replays_the_perplexity_replies_corpus(
    #[files("fuzz/corpus/perplexity_replies/*")] path: PathBuf,
) {
    let body = fs::read(path).expect("the corpus fixture");
    let _embedding = transform_embedding_response("pplx-embed-v1-0.6b", 200, &body);
    let _search = decode_search_reply(&body);
}
