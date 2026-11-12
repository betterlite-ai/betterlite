#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::vector_stores::config::{capabilities, validate_environment};
use crate::vector_stores::endpoint::{CompleteUrl, complete_url, retrieve_url};
use crate::vector_stores::error::VectorStoreError;
use crate::vector_stores::filters::{map_and_or_filters, map_operator_filter, map_operator_to_aws};
use crate::vector_stores::metadata::{file_id_from_metadata, filename_from_metadata};
use crate::vector_stores::request::{
    SearchQuery, SearchRequest, build_search_request, user_context,
};
use crate::vector_stores::tests::support::{header, json, object, optional, optional_params, sign};

use super::responses::{
    attributes_and_scores, knowledge_base_response_error, option_mapping, retrieval_configuration,
    unsupported_create,
};

#[rstest]
#[case::filter_operator_map(filter_operator_map)]
#[case::nested_and_or_filters(nested_and_or_filters)]
#[case::knowledge_base_retrieve_url(knowledge_base_retrieve_url)]
#[case::user_context(user_context_case)]
#[case::location_file_id_name(location_file_id_name)]
#[case::attributes_and_scores(attributes_and_scores)]
#[case::unsupported_create(unsupported_create)]
#[case::option_mapping(option_mapping)]
#[case::retrieval_configuration(retrieval_configuration)]
#[case::knowledge_base_response_error(knowledge_base_response_error)]
fn vector_stores_cases(#[case] scenario: fn()) {
    scenario();
}

fn filter_operator_map() {
    operator_mapping();
    single_filter_mapping();
}

fn operator_mapping() {
    let operator_names = "eq ne gt gte lt lte in nin equals notEquals greaterThan \
        greaterThanOrEquals lessThan lessThanOrEquals notIn startsWith listContains \
        stringContains custom";
    let aws_names = "equals notEquals greaterThan greaterThanOrEquals lessThan \
        lessThanOrEquals in notIn equals notEquals greaterThan greaterThanOrEquals \
        lessThan lessThanOrEquals notIn startsWith listContains stringContains custom";
    let mapped: Vec<&str> = operator_names
        .split_whitespace()
        .map(map_operator_to_aws)
        .collect();
    assert_eq!(mapped, aws_names.split_whitespace().collect::<Vec<_>>());
}

fn single_filter_mapping() {
    let single = map_operator_filter(&json(
        r#"{"operator":"eq","key":"genre","value":"fiction","extra":true}"#,
    ))
    .unwrap();
    assert_eq!(
        single,
        json(r#"{"equals":{"key":"genre","value":"fiction"}}"#)
    );
    assert!(matches!(
        map_operator_filter(&json(r#"{"operator":"eq","key":"genre"}"#)),
        Err(VectorStoreError::FilterMissingField { field: "value" })
    ));
    assert!(matches!(
        map_operator_filter(&json(r#"{"key":"genre","value":"x"}"#)),
        Err(VectorStoreError::FilterMissingField { field: "operator" })
    ));
    assert!(matches!(
        map_operator_filter(&json(r#"["x"]"#)),
        Err(VectorStoreError::FilterNotObject)
    ));
}

fn nested_and_or_filters() {
    compound_single_children();
    compound_lists();
    compound_errors();
}

fn compound_single_children() {
    let single_and = map_and_or_filters(&json(
        r#"{"and":[{"operator":"eq","key":"k","value":"v"}],"or":[{"operator":"ne","key":"k","value":"w"}]}"#,
    ))
    .unwrap();
    assert_eq!(single_and, json(r#"{"equals":{"key":"k","value":"v"}}"#));
    let single_or = map_and_or_filters(&json(
        r#"{"and":[{"operator":"eq","key":"a","value":1},{"operator":"gt","key":"b","value":2}],"or":[{"operator":"eq","key":"c","value":3}]}"#,
    ))
    .unwrap();
    assert_eq!(single_or, json(r#"{"equals":{"key":"c","value":3}}"#));
}

fn compound_lists() {
    let both = map_and_or_filters(&json(
        r#"{"and":[{"operator":"eq","key":"a","value":1},{"operator":"gt","key":"b","value":2}],"or":[{"operator":"lt","key":"c","value":3},{"operator":"startsWith","key":"d","value":"x"}]}"#,
    ))
    .unwrap();
    assert_eq!(
        both,
        json(
            r#"{"andAll":[{"equals":{"key":"a","value":1}},{"greaterThan":{"key":"b","value":2}}],"orAll":[{"lessThan":{"key":"c","value":3}},{"startsWith":{"key":"d","value":"x"}}]}"#
        )
    );
    assert_eq!(
        map_and_or_filters(&json(r#"{"and":[]}"#)).unwrap(),
        json(r#"{"andAll":[]}"#)
    );
    assert_eq!(
        map_and_or_filters(&json(r#"{"or":[]}"#)).unwrap(),
        json(r#"{"orAll":[]}"#)
    );
    assert_eq!(map_and_or_filters(&json("{}")).unwrap(), json("{}"));
}

fn compound_errors() {
    assert!(matches!(
        map_and_or_filters(&json(r#"{"and":[{"and":[]}]}"#)),
        Err(VectorStoreError::FilterMissingField { field: "operator" })
    ));
    assert!(matches!(
        map_and_or_filters(&json(r#"{"and":"x"}"#)),
        Err(VectorStoreError::FilterListNotArray { field: "and" })
    ));
    assert!(matches!(
        map_and_or_filters(&json(r#"{"or":[5]}"#)),
        Err(VectorStoreError::FilterNotObject)
    ));
}

fn complete_url_for(
    api_base: Option<&str>,
    configured: Option<&str>,
    environment: Option<&str>,
    region: Option<&str>,
    environment_region: Option<&str>,
) -> Result<String, VectorStoreError> {
    complete_url(&CompleteUrl {
        api_base,
        configured_endpoint: configured,
        environment_endpoint: environment,
        aws_region_name: region,
        environment_region_name: None,
        environment_region,
    })
}

fn knowledge_base_retrieve_url() {
    let capabilities = capabilities();
    assert_eq!(
        capabilities.supported_params,
        ["filters", "max_num_results", "ranking_options"]
    );
    assert_eq!(
        capabilities.endpoints.read,
        Some(vec![(
            "POST".to_owned(),
            "/knowledgebases/{knowledge_base_id}/retrieve".to_owned(),
        )])
    );
    assert_eq!(capabilities.endpoints.write, Some(Vec::new()));
    assert_eq!(
        serde_json::to_string(&capabilities.credentials).unwrap(),
        "{}"
    );
    assert_endpoint_urls();
    assert_environment_headers();
    assert_retrieve_paths();
    assert_signed_requests();
}

fn assert_endpoint_urls() {
    assert_default_endpoints();
    assert_configured_endpoints();
}

fn assert_default_endpoints() {
    assert_eq!(
        complete_url_for(None, None, None, Some("us-east-1"), None).unwrap(),
        "https://bedrock-agent-runtime.us-east-1.amazonaws.com/knowledgebases"
    );
    assert_eq!(
        complete_url_for(None, None, None, None, Some("eu-central-1")).unwrap(),
        "https://bedrock-agent-runtime.eu-central-1.amazonaws.com/knowledgebases"
    );
    assert_eq!(
        complete_url_for(None, None, None, None, None).unwrap(),
        "https://bedrock-agent-runtime.us-west-2.amazonaws.com/knowledgebases"
    );
    complete_url_for(None, None, None, Some("US-EAST-1"), None).unwrap_err();
}

fn assert_configured_endpoints() {
    assert_eq!(
        complete_url_for(
            Some("https://custom.example"),
            Some("https://configured.example"),
            Some("https://environment.example"),
            Some("us-east-1"),
            Some("eu-central-1")
        )
        .unwrap(),
        "https://custom.example/knowledgebases"
    );
    assert_eq!(
        complete_url_for(
            None,
            Some("https://configured.example/"),
            Some("https://environment.example"),
            Some("us-east-1"),
            None
        )
        .unwrap(),
        "https://configured.example//knowledgebases"
    );
    assert_eq!(
        complete_url_for(
            None,
            None,
            Some("https://environment.example/"),
            Some("us-east-1"),
            None
        )
        .unwrap(),
        "https://environment.example//knowledgebases"
    );
}

fn assert_environment_headers() {
    assert_eq!(
        validate_environment(BTreeMap::new()),
        BTreeMap::from([(
            "Content-Type".to_owned(),
            Some("application/json".to_owned())
        )])
    );
    assert_eq!(
        validate_environment(BTreeMap::from([(
            "Content-Type".to_owned(),
            Some("text/plain".to_owned()),
        )])),
        BTreeMap::from([("Content-Type".to_owned(), Some("text/plain".to_owned()))])
    );
    let lowercased = validate_environment(BTreeMap::from([(
        "content-type".to_owned(),
        Some("text/plain".to_owned()),
    )]));
    assert_eq!(lowercased.len(), 2);
}

fn assert_retrieve_paths() {
    assert_eq!(
        retrieve_url("https://host/knowledgebases", "kb-1").unwrap(),
        "https://host/knowledgebases/kb-1/retrieve"
    );
    assert_eq!(
        retrieve_url("https://host/knowledgebases", "kb/one").unwrap(),
        "https://host/knowledgebases/kb%2Fone/retrieve"
    );
    retrieve_url("https://host/knowledgebases", "").unwrap_err();
    retrieve_url("https://host/knowledgebases", ".").unwrap_err();
}

fn assert_signed_requests() {
    let data = object(r#"{"retrievalQuery":{"text":"hello"}}"#);
    let headers = [(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let bearer_key = SecretString::from("fixture-bearer-key");
    let bearer = sign(&headers, &BTreeMap::new(), &data, Some(&bearer_key)).unwrap();
    assert_eq!(
        header(&bearer, "authorization"),
        "Bearer fixture-bearer-key"
    );
    assert_eq!(
        bearer.body,
        br#"{"retrievalQuery": {"text": "hello"}}"#.to_vec()
    );
    let params = optional_params(&[("aws_region_name", r#""us-east-1""#)]);
    let sigv4 = sign(&headers, &params, &data, None).unwrap();
    let authorization = header(&sigv4, "authorization");
    assert!(
        authorization.contains("/us-east-1/bedrock/aws4_request"),
        "{authorization}"
    );
    assert!(!authorization.contains("bedrock-agent-runtime"));
    assert_eq!(header(&sigv4, "x-amz-date"), "20231114T221320Z");
}

fn user_context_case() {
    user_context_sources();
    user_context_falsey_values();
    user_context_in_request();
}

fn user_context_sources() {
    let params = object(r#"{"userContext":{"userId":"camel"},"user_context":{"userId":"snake"}}"#);
    assert_eq!(
        user_context(None, &params),
        Some(json(r#"{"userId":"camel"}"#))
    );
    assert_eq!(
        user_context(Some(&object("{}")), &params),
        Some(json(r#"{"userId":"camel"}"#))
    );
    assert_eq!(
        user_context(
            Some(&object(r#"{"userContext":{"userId":"extra"}}"#)),
            &params
        ),
        Some(json(r#"{"userId":"extra"}"#))
    );
    assert_eq!(
        user_context(
            None,
            &object(r#"{"userContext":null,"user_context":{"userId":"snake"}}"#)
        ),
        Some(json(r#"{"userId":"snake"}"#))
    );
    assert_eq!(
        user_context(
            Some(&object(r#"{"userContext":null}"#)),
            &object(r#"{"user_context":"value"}"#)
        ),
        Some(json(r#""value""#))
    );
}

fn user_context_falsey_values() {
    assert_eq!(
        user_context(None, &object(r#"{"userContext":{}}"#)),
        Some(json("{}"))
    );
    assert_eq!(
        user_context(None, &object(r#"{"userContext":false}"#)),
        Some(Json::Bool(false))
    );
    assert_eq!(
        user_context(None, &object(r#"{"userContext":""}"#)),
        Some(json(r#""""#))
    );
    assert_eq!(
        user_context(None, &object(r#"{"user_context":null}"#)),
        None
    );
    assert_eq!(user_context(None, &object("{}")), None);
}

fn user_context_in_request() {
    let call = build_search_request(&SearchRequest {
        vector_store_id: "kb-1",
        query: &SearchQuery::Text("a question".to_owned()),
        optional: &optional(None, None),
        api_base: "https://host/knowledgebases",
        litellm_params: &object(r#"{"other":1}"#),
        extra_body: Some(&object(
            r#"{"userContext":{"userId":"reader"},"retrievalConfiguration":{"vectorSearchConfiguration":{"numberOfResults":7}},"ignored":true}"#,
        )),
    })
    .unwrap();
    assert_eq!(
        call.body.get("userContext"),
        Some(&json(r#"{"userId":"reader"}"#))
    );
    assert!(!call.body.contains_key("ignored"));
    assert_eq!(call.query, "a question");
}

const FILE_ID_CASES: &[(&str, &str)] = &[
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir/report.txt"}"#,
        r#""s3://bucket/dir/report.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-chunk-id":"chunk-1"}"#,
        r#""bedrock-kb-chunk-1""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-chunk-id":null}"#,
        r#""bedrock-kb-None""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":""}"#,
        r#""bedrock-kb-unknown""#,
    ),
];

const FILENAME_CASES: &[(&str, &str)] = &[
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir/report.txt"}"#,
        r#""report.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir/"}"#,
        r#""bucket""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket"}"#,
        r#""bucket""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"https://example.com/a/b.pdf"}"#,
        r#""b.pdf""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/a%20b.txt"}"#,
        r#""a%20b.txt""#,
    ),
    (r#"{"x-amz-bedrock-kb-source-uri":"a/b"}"#, r#""b""#),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[1.2.3.4]/x"}"#,
        r#""http://[1.2.3.4]/x""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":" s3://bucket/dir/"}"#,
        r#""bucket""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"\u0001s3://bucket/k.txt"}"#,
        r#""k.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[::1]/box.txt"}"#,
        r#""box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[fe80::1%eth0]/box.txt"}"#,
        r#""box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[fe80::1%]/box.txt"}"#,
        r#""http://[fe80::1%]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[v1.x]/box.txt"}"#,
        r#""box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[v1.]/box.txt"}"#,
        r#""http://[v1.]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[vz.x]/box.txt"}"#,
        r#""http://[vz.x]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[v.x]/box.txt"}"#,
        r#""http://[v.x]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"https://host/a;b.pdf"}"#,
        r#""a""#,
    ),
    (r#"{"x-amz-bedrock-kb-source-uri":"a/b;c"}"#, r#""b""#),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"https://host/x;"}"#,
        r#""x""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"https://host/;x"}"#,
        r#""host""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/a;b.txt"}"#,
        r#""a;b.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"HTTPS://host/a;b"}"#,
        r#""a""#,
    ),
    (r#"{"x-amz-bedrock-kb-source-uri":"a;b"}"#, r#""a""#),
    (r#"{"x-amz-bedrock-kb-source-uri":";x"}"#, r#""""#),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"ftp://h/d;t/e;f.txt"}"#,
        r#""e""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"https://host/dir;x/"}"#,
        r#""host""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"https://host/a;b/c"}"#,
        r#""c""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://host;x"}"#,
        r#""host;x""#,
    ),
    (r#"{"x-amz-bedrock-kb-source-uri":"/;x"}"#, r#""""#),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir;x/"}"#,
        r#""bucket""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://u@[::1]:80/box.txt"}"#,
        r#""box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://a[::1]/box.txt"}"#,
        r#""http://a[::1]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[::1]x/box.txt"}"#,
        r#""http://[::1]x/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[]/box.txt"}"#,
        r#""http://[]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[::1]:80/box.txt"}"#,
        r#""box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://u[x@host]/box.txt"}"#,
        r#""http://u[x@host]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[::1%a%b]/box.txt"}"#,
        r#""http://[::1%a%b]/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[::1]:80@x/box.txt"}"#,
        r#""http://[::1]:80@x/box.txt""#,
    ),
    (
        r#"{"x-amz-bedrock-kb-source-uri":"http://[g::1]/box.txt"}"#,
        r#""http://[g::1]/box.txt""#,
    ),
];

fn location_file_id_name() {
    assert_metadata_projections(file_id_from_metadata, FILE_ID_CASES);
    assert_metadata_projections(filename_from_metadata, FILENAME_CASES);
    metadata_fallbacks();
}

fn assert_metadata_projections(
    project: fn(&IndexMap<String, Json>) -> Json,
    cases: &[(&str, &str)],
) {
    for (metadata, expected) in cases {
        assert_eq!(project(&object(metadata)), json(expected));
    }
}

fn metadata_fallbacks() {
    assert_eq!(
        file_id_from_metadata(&object("{}")),
        json(r#""bedrock-kb-unknown""#)
    );
    assert_eq!(
        filename_from_metadata(&object(
            r#"{"x-amz-bedrock-kb-source-uri":"http://[::1/box.txt"}"#
        )),
        json(r#""http://[::1/box.txt""#)
    );
    assert_eq!(
        filename_from_metadata(&object(r#"{"x-amz-bedrock-kb-data-source-id":"ds-1"}"#)),
        json(r#""bedrock-kb-document-ds-1""#)
    );
    assert_eq!(
        filename_from_metadata(&object("{}")),
        json(r#""bedrock-kb-document-unknown""#)
    );
    assert_eq!(
        file_id_from_metadata(&object(r#"{"x-amz-bedrock-kb-source-uri":0}"#)),
        json(r#""bedrock-kb-unknown""#)
    );
    assert_eq!(
        filename_from_metadata(&object(r#"{"x-amz-bedrock-kb-source-uri":"a b://host/"}"#)),
        json(r#""""#)
    );
}
