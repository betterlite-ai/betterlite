#![cfg(test)]

use std::collections::BTreeMap;
use std::future::ready;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::value::RawValue;

use crate::aws_settings::sources::Credentials;
use crate::bedrock_settings::ErrorHeaders;
use crate::credentials::error::CredentialError;
use crate::signing::request::AwsSignedRequest;
use crate::signing::target::RegionInputs;
use crate::vector_stores::config::{capabilities, validate_environment};
use crate::vector_stores::create::{CreateStep, transform_create_vector_store};
use crate::vector_stores::endpoint::{CompleteUrl, complete_url, retrieve_url};
use crate::vector_stores::error::VectorStoreError;
use crate::vector_stores::filters::{
    map_and_or_filters, map_openai_params, map_operator_filter, map_operator_to_aws,
};
use crate::vector_stores::metadata::{file_id_from_metadata, filename_from_metadata};
use crate::vector_stores::request::{
    SearchCall, SearchOptionalParams, SearchQuery, SearchRequest, build_search_request,
    user_context,
};
use crate::vector_stores::response::decode_knowledge_base_reply;
use crate::vector_stores::signing::{SignRequest, sign_request};
use crate::vector_stores::tests::support::{
    capture, json, now, object, optional, optional_params, runtime, sign,
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

fn header<'signed>(signed: &'signed AwsSignedRequest, name: &str) -> &'signed str {
    signed
        .headers
        .iter()
        .find(|(found, _)| found.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
        .unwrap()
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

fn location_file_id_name() {
    file_id_sources();
    filename_sources();
    metadata_fallbacks();
}

fn file_id_sources() {
    assert_eq!(
        file_id_from_metadata(&object(
            r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir/report.txt"}"#
        )),
        json(r#""s3://bucket/dir/report.txt""#)
    );
    assert_eq!(
        file_id_from_metadata(&object(r#"{"x-amz-bedrock-kb-chunk-id":"chunk-1"}"#)),
        json(r#""bedrock-kb-chunk-1""#)
    );
    assert_eq!(
        file_id_from_metadata(&object(r#"{"x-amz-bedrock-kb-chunk-id":null}"#)),
        json(r#""bedrock-kb-None""#)
    );
    assert_eq!(
        file_id_from_metadata(&object(r#"{"x-amz-bedrock-kb-source-uri":""}"#)),
        json(r#""bedrock-kb-unknown""#)
    );
}

fn filename_sources() {
    assert_eq!(
        filename_from_metadata(&object(
            r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir/report.txt"}"#
        )),
        json(r#""report.txt""#)
    );
    assert_eq!(
        filename_from_metadata(&object(
            r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/dir/"}"#
        )),
        json(r#""bucket""#)
    );
    assert_eq!(
        filename_from_metadata(&object(r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket"}"#)),
        json(r#""bucket""#)
    );
    assert_eq!(
        filename_from_metadata(&object(
            r#"{"x-amz-bedrock-kb-source-uri":"https://example.com/a/b.pdf"}"#
        )),
        json(r#""b.pdf""#)
    );
    assert_eq!(
        filename_from_metadata(&object(
            r#"{"x-amz-bedrock-kb-source-uri":"s3://bucket/a%20b.txt"}"#
        )),
        json(r#""a%20b.txt""#)
    );
    assert_eq!(
        filename_from_metadata(&object(r#"{"x-amz-bedrock-kb-source-uri":"a/b"}"#)),
        json(r#""b""#)
    );
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

fn decode(body: &str, status: u16) -> Result<Json, VectorStoreError> {
    decode_knowledge_base_reply(status, &[], body.as_bytes(), "question")
        .map(|response| json(&serde_json::to_string(&response).unwrap()))
}

fn attributes_and_scores() {
    let body = r#"{"retrievalResults":[
      {"content":{"type":"text","text":"first"},"score":0.5,"metadata":{"x-amz-bedrock-kb-source-uri":"s3://bucket/one.txt","x-amz-bedrock-kb-chunk-id":"c1","custom":"a"},"location":{"type":"S3","s3Location":{"uri":"s3://elsewhere/ignored.txt"}}},
      {"content":{"type":"text","text":""},"metadata":{}},
      {"content":{},"score":1},
      {"content":{"text":null},"score":1},
      {"score":1},
      {"content":{"text":"third"},"metadata":{"x-amz-bedrock-kb-source-uri":"https://example.com/two.pdf"}},
      {"content":{"text":"fourth"},"score":2},
      {"content":{"text":"fifth"},"metadata":null}
    ]}"#;
    let expected = json(
        r#"{"object":"vector_store.search_results.page","search_query":"question","data":[
          {"score":0.5,"content":[{"text":"first","type":"text"}],"file_id":"s3://bucket/one.txt","filename":"one.txt","attributes":{"x-amz-bedrock-kb-source-uri":"s3://bucket/one.txt","x-amz-bedrock-kb-chunk-id":"c1","custom":"a"}},
          {"score":null,"content":[{"text":"","type":"text"}],"file_id":"bedrock-kb-unknown","filename":"bedrock-kb-document-unknown","attributes":{}},
          {"score":null,"content":[{"text":"third","type":"text"}],"file_id":"https://example.com/two.pdf","filename":"two.pdf","attributes":{"x-amz-bedrock-kb-source-uri":"https://example.com/two.pdf"}},
          {"score":2,"content":[{"text":"fourth","type":"text"}],"file_id":"bedrock-kb-unknown","filename":"bedrock-kb-document-unknown","attributes":{}},
          {"score":null,"content":[{"text":"fifth","type":"text"}],"file_id":"bedrock-kb-unknown","filename":"bedrock-kb-document-unknown","attributes":{}}
        ]}"#,
    );
    assert_eq!(decode(body, 200).unwrap(), expected);
}

fn unsupported_create() {
    assert!(matches!(
        transform_create_vector_store(CreateStep::Request),
        Err(VectorStoreError::CreateNotSupported)
    ));
    assert!(matches!(
        transform_create_vector_store(CreateStep::Response),
        Err(VectorStoreError::CreateNotSupported)
    ));
    assert_eq!(capabilities().endpoints.write, Some(Vec::new()));
}

fn option_mapping() {
    mapped_openai_params();
    filter_shapes();
    ignored_params();
}

fn mapped_openai_params() {
    let mut params = object(r#"{"filters":{"old":1},"kept":true}"#);
    let non_default = object(
        r#"{"max_num_results":3,"filters":{"operator":"gte","key":"year","value":2020},"ranking_options":{"ranker":"default"},"drop_params":true}"#,
    );
    map_openai_params(&non_default, &mut params).unwrap();
    assert_eq!(
        params,
        object(
            r#"{"filters":{"greaterThanOrEquals":{"key":"year","value":2020}},"kept":true,"numberOfResults":3}"#
        )
    );
    let mut null_max = object("{}");
    map_openai_params(&object(r#"{"max_num_results":null}"#), &mut null_max).unwrap();
    assert_eq!(null_max, object(r#"{"numberOfResults":null}"#));
    let mut kept = object(r#"{"filters":"kept"}"#);
    map_openai_params(&object(r#"{"filters":null}"#), &mut kept).unwrap();
    assert_eq!(kept, object(r#"{"filters":"kept"}"#));
}

fn filter_shapes() {
    let mut not_a_dict = object("{}");
    map_openai_params(&object(r#"{"filters":"oops"}"#), &mut not_a_dict).unwrap();
    assert_eq!(not_a_dict, object(r#"{"filters":null}"#));
    let mut compound = object("{}");
    map_openai_params(
        &object(
            r#"{"filters":{"and":[{"operator":"eq","key":"a","value":1},{"operator":"eq","key":"b","value":2}]}}"#,
        ),
        &mut compound,
    )
    .unwrap();
    assert_eq!(
        compound,
        object(
            r#"{"filters":{"andAll":[{"equals":{"key":"a","value":1}},{"equals":{"key":"b","value":2}}]}}"#
        )
    );
    let mut aws_shaped = object("{}");
    map_openai_params(
        &object(r#"{"filters":{"equals":{"key":"a","value":1}}}"#),
        &mut aws_shaped,
    )
    .unwrap();
    assert_eq!(
        aws_shaped,
        object(r#"{"filters":{"equals":{"key":"a","value":1}}}"#)
    );
}

fn ignored_params() {
    let mut ignored = object("{}");
    map_openai_params(
        &object(r#"{"ranking_options":{"a":1},"rewrite_query":true,"temperature":0.2}"#),
        &mut ignored,
    )
    .unwrap();
    assert_eq!(ignored, object("{}"));
    map_openai_params(
        &object(r#"{"filters":{"operator":"eq","key":"a"}}"#),
        &mut object("{}"),
    )
    .unwrap_err();
}

fn search(
    query: &SearchQuery,
    optional_params: &SearchOptionalParams,
    extra_body: Option<&IndexMap<String, Json>>,
) -> Result<SearchCall, VectorStoreError> {
    build_search_request(&SearchRequest {
        vector_store_id: "kb-1",
        query,
        optional: optional_params,
        api_base: "https://host/knowledgebases",
        litellm_params: &object("{}"),
        extra_body,
    })
}

fn retrieval_configuration() {
    query_joining();
    copied_configuration();
    overridden_configuration();
    configuration_errors();
}

fn query_joining() {
    let call = search(
        &SearchQuery::TextList(vec!["one".to_owned(), "two".to_owned()]),
        &optional(None, None),
        None,
    )
    .unwrap();
    assert_eq!(call.url, "https://host/knowledgebases/kb-1/retrieve");
    assert_eq!(call.query, "one two");
    assert_eq!(
        call.body,
        object(r#"{"retrievalQuery":{"text":"one two"}}"#)
    );
    let empty = search(
        &SearchQuery::TextList(Vec::new()),
        &optional(None, None),
        None,
    )
    .unwrap();
    assert_eq!(empty.query, "");
}

fn copied_configuration() {
    let call = search(
        &SearchQuery::Text("kept".to_owned()),
        &optional(None, None),
        Some(&object(
            r#"{"retrievalConfiguration":{"vectorSearchConfiguration":{"numberOfResults":5,"filter":{"equals":{"key":"k","value":"v"}}},"custom":true}}"#,
        )),
    )
    .unwrap();
    assert_eq!(
        call.body.get("retrievalConfiguration"),
        Some(&json(
            r#"{"vectorSearchConfiguration":{"numberOfResults":5,"filter":{"equals":{"key":"k","value":"v"}}},"custom":true}"#
        ))
    );
    let fallback = search(
        &SearchQuery::Text("fallback".to_owned()),
        &optional(None, None),
        Some(&object(
            r#"{"retrievalConfiguration":{},"retrieval_configuration":{"vectorSearchConfiguration":{"numberOfResults":2}}}"#,
        )),
    )
    .unwrap();
    assert_eq!(
        fallback.body.get("retrievalConfiguration"),
        Some(&json(
            r#"{"vectorSearchConfiguration":{"numberOfResults":2}}"#
        ))
    );
    let empty = search(
        &SearchQuery::Text("empty".to_owned()),
        &optional(None, None),
        Some(&object(r#"{"retrievalConfiguration":{}}"#)),
    )
    .unwrap();
    assert!(!empty.body.contains_key("retrievalConfiguration"));
}

fn overridden_configuration() {
    let call = search(
        &SearchQuery::Text("override".to_owned()),
        &optional(Some(json(r#"{"equals":{"key":"k","value":"new"}}"#)), Some(2)),
        Some(&object(
            r#"{"retrievalConfiguration":{"vectorSearchConfiguration":{"numberOfResults":5,"filter":{"equals":{"key":"k","value":"old"}}},"unknown":"kept"},"retrieval_configuration":{"vectorSearchConfiguration":{"numberOfResults":9}}}"#,
        )),
    )
    .unwrap();
    assert_eq!(
        call.body.get("retrievalConfiguration"),
        Some(&json(
            r#"{"vectorSearchConfiguration":{"numberOfResults":2,"filter":{"equals":{"key":"k","value":"new"}}},"unknown":"kept"}"#
        ))
    );
    let created = search(
        &SearchQuery::Text("create".to_owned()),
        &optional(Some(json("{}")), Some(4)),
        None,
    )
    .unwrap();
    assert_eq!(
        created.body.get("retrievalConfiguration"),
        Some(&json(
            r#"{"vectorSearchConfiguration":{"numberOfResults":4,"filter":{}}}"#
        ))
    );
}

fn configuration_errors() {
    assert!(matches!(
        search(
            &SearchQuery::Text("aws".to_owned()),
            &optional(Some(json(r#"{"equals":{"key":"k","value":"v"}}"#)), None),
            Some(&object(r#"{"retrievalConfiguration":"raw"}"#)),
        ),
        Err(VectorStoreError::RetrievalConfigNotObject)
    ));
    assert!(matches!(
        search(
            &SearchQuery::Text("null config".to_owned()),
            &optional(None, Some(1)),
            Some(&object(
                r#"{"retrievalConfiguration":{"vectorSearchConfiguration":null}}"#,
            )),
        ),
        Err(VectorStoreError::SearchConfigurationNotObject)
    ));
    let call = search(
        &SearchQuery::Text("raw".to_owned()),
        &optional(None, None),
        Some(&object(r#"{"retrievalConfiguration":"raw"}"#)),
    )
    .unwrap();
    assert_eq!(
        call.body.get("retrievalConfiguration"),
        Some(&json(r#""raw""#))
    );
}

#[test]
fn override_diagnostics_name_the_source() {
    let (captured, _guard) = capture();
    search(
        &SearchQuery::Text("q".to_owned()),
        &optional(None, Some(2)),
        Some(&object(
            r#"{"retrievalConfiguration":{"vectorSearchConfiguration":{"numberOfResults":5}}}"#,
        )),
    )
    .unwrap();
    search(
        &SearchQuery::Text("q".to_owned()),
        &optional(Some(json(r#"{"equals":{"key":"k","value":"new"}}"#)), None),
        Some(&object(
            r#"{"retrievalConfiguration":{"vectorSearchConfiguration":{"filter":{"equals":{"key":"k","value":"old"}}}}}"#,
        )),
    )
    .unwrap();
    let text = captured.text();
    assert!(
        text.contains(
            "Overriding extra_body retrievalConfiguration.vectorSearchConfiguration.numberOfResults (5) with max_num_results=2"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "Overriding extra_body retrievalConfiguration.vectorSearchConfiguration.filter with filters from vector_store_search_optional_params"
        ),
        "{text}"
    );
}

#[test]
fn override_diagnostics_need_a_different_prior_value() {
    assert_no_override_log(
        r#"{"retrievalConfiguration":{"vectorSearchConfiguration":{"numberOfResults":2,"filter":{"equals":{"key":"k","value":"same"}}}}}"#,
        Some(json(r#"{"equals":{"key":"k","value":"same"}}"#)),
        Some(2),
    );
    assert_no_override_log(
        r#"{"retrievalConfiguration":{"vectorSearchConfiguration":{"numberOfResults":null,"filter":null}}}"#,
        Some(json(r#"{"equals":{"key":"k","value":"new"}}"#)),
        Some(2),
    );
}

fn assert_no_override_log(extra_body: &str, filters: Option<Json>, max: Option<i64>) {
    let (captured, _guard) = capture();
    search(
        &SearchQuery::Text("q".to_owned()),
        &optional(filters, max),
        Some(&object(extra_body)),
    )
    .unwrap();
    let text = captured.text();
    assert!(!text.contains("Overriding extra_body"), "{text}");
}

fn knowledge_base_response_error() {
    decode_errors();
    falsey_results();
    non_200_success();
}

fn decode_errors() {
    assert_provider_status(&decode("{", 200).unwrap_err(), 200);
    assert_provider_status(&decode("[]", 200).unwrap_err(), 200);
    assert_provider_status(
        &decode(r#"{"retrievalResults":"x"}"#, 503).unwrap_err(),
        503,
    );
    assert_provider_status(
        &decode(r#"{"retrievalResults":["x"]}"#, 503).unwrap_err(),
        503,
    );
    assert_provider_status(
        &decode(r#"{"retrievalResults":[{"content":"x"}]}"#, 503).unwrap_err(),
        503,
    );
    assert_provider_status(
        &decode(
            r#"{"retrievalResults":[{"content":{"text":"x"},"metadata":["x"]}]}"#,
            503,
        )
        .unwrap_err(),
        503,
    );
    let headers = [("X-Fixture".to_owned(), "present".to_owned())];
    let VectorStoreError::Provider { error, .. } =
        decode_knowledge_base_reply(200, &headers, b"{", "q").unwrap_err()
    else {
        panic!("expected a provider error");
    };
    assert_eq!(
        error.cause.headers,
        Some(ErrorHeaders::Http(headers.to_vec()))
    );
}

fn falsey_results() {
    let empty_page = json(
        r#"{"object":"vector_store.search_results.page","search_query":"question","data":[]}"#,
    );
    for missing in [
        r#"{"nextToken":"n"}"#,
        r#"{"retrievalResults":null}"#,
        r#"{"retrievalResults":[]}"#,
        r#"{"retrievalResults":0}"#,
        r#"{"retrievalResults":{}}"#,
    ] {
        assert_eq!(decode(missing, 200).unwrap(), empty_page, "{missing}");
    }
}

fn non_200_success() {
    assert_eq!(
        decode(r#"{"retrievalResults":[]}"#, 500).unwrap(),
        json(
            r#"{"object":"vector_store.search_results.page","search_query":"question","data":[]}"#
        )
    );
}

fn assert_provider_status(error: &VectorStoreError, status: u16) {
    let VectorStoreError::Provider {
        status: found,
        error: provider,
    } = error
    else {
        panic!("expected a provider error, got {error:?}");
    };
    assert_eq!(*found, status);
    assert!(!provider.cause.error.status_code_is_synthesized);
}

#[test]
fn knowledge_base_credential_identity_isolation() {
    bearer_identity();

    let credentials = Credentials {
        access_key_id: SecretString::from("AKIAIDENTITYAAAAAAAA"),
        secret_access_key: SecretString::from("first-secret"),
        session_token: None,
    };
    let other = Credentials {
        access_key_id: SecretString::from("AKIAIDENTITYBBBBBBBB"),
        secret_access_key: SecretString::from("second-secret"),
        session_token: None,
    };
    let (first, second) = signed_identities(&credentials, &other);
    let first_sig = header(&first, "authorization");
    let second_sig = header(&second, "authorization");
    assert!(first_sig.contains("AKIAIDENTITYAAAAAAAA"));
    assert!(second_sig.contains("AKIAIDENTITYBBBBBBBB"));
    assert!(!second_sig.contains("AKIAIDENTITYAAAAAAAA"));
    assert_ne!(first_sig, second_sig);
}

fn bearer_identity() {
    let data = object(r#"{"retrievalQuery":{"text":"identity"}}"#);
    let headers = [(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let first = sign(
        &headers,
        &BTreeMap::new(),
        &data,
        Some(&SecretString::from("fixture-key-tenant-a")),
    )
    .unwrap();
    let second = sign(
        &headers,
        &BTreeMap::new(),
        &data,
        Some(&SecretString::from("fixture-key-tenant-b")),
    )
    .unwrap();
    assert_eq!(
        header(&first, "authorization"),
        "Bearer fixture-key-tenant-a"
    );
    assert_eq!(
        header(&second, "authorization"),
        "Bearer fixture-key-tenant-b"
    );
    assert!(!header(&second, "authorization").contains("tenant-a"));
    let first_headers = validate_environment(BTreeMap::from([(
        "X-Identity".to_owned(),
        Some("a".to_owned()),
    )]));
    assert_eq!(first_headers.len(), 2);
    assert!(!validate_environment(BTreeMap::new()).contains_key("X-Identity"));
}

fn signed_identities(
    first: &Credentials,
    second: &Credentials,
) -> (AwsSignedRequest, AwsSignedRequest) {
    let data = object(r#"{"retrievalQuery":{"text":"identity"}}"#);
    let headers = [(
        "Content-Type".to_owned(),
        Some("application/json".to_owned()),
    )];
    let params = optional_params(&[("aws_region_name", r#""us-east-1""#)]);
    (
        sign_with_credentials(&headers, &params, &data, first),
        sign_with_credentials(&headers, &params, &data, second),
    )
}

fn sign_with_credentials(
    headers: &[(String, Option<String>)],
    params: &BTreeMap<String, Box<RawValue>>,
    data: &IndexMap<String, Json>,
    credentials: &Credentials,
) -> AwsSignedRequest {
    runtime()
        .block_on(sign_request(
            SignRequest {
                headers,
                optional_params: params,
                request_data: data,
                api_base: "https://host/knowledgebases",
                api_key: None,
                configured_bearer: None,
                region_inputs: RegionInputs::default(),
                now: now(),
            },
            |_settings, _region| ready(Ok::<Credentials, _>(credentials.clone())),
        ))
        .unwrap()
}

#[test]
fn knowledge_base_secret_safe_failure() {
    assert_decode_failure_is_secret_safe();
    assert_signing_failure_is_secret_safe();
}

fn assert_decode_failure_is_secret_safe() {
    let fixture_secret = "fixture-secret-value-9f2c";
    let headers = [
        (
            "Authorization".to_owned(),
            format!("Bearer {fixture_secret}"),
        ),
        ("X-Credential".to_owned(), fixture_secret.to_owned()),
    ];
    let error = decode_knowledge_base_reply(200, &headers, b"{", "q").unwrap_err();
    for rendered in [format!("{error:?}"), format!("{error}")] {
        assert!(!rendered.contains(fixture_secret), "{rendered}");
    }
}

fn assert_signing_failure_is_secret_safe() {
    let fixture_secret = "fixture-secret-value-9f2c";
    let signing_headers = [(
        "Authorization".to_owned(),
        Some(format!("Bearer {fixture_secret}")),
    )];
    let params = optional_params(&[("aws_region_name", r#""us-east-1""#)]);
    let data = object(r#"{"retrievalQuery":{"text":"secret"}}"#);
    let error = runtime()
        .block_on(sign_request(
            SignRequest {
                headers: &signing_headers,
                optional_params: &params,
                request_data: &data,
                api_base: "https://host/knowledgebases",
                api_key: None,
                configured_bearer: None,
                region_inputs: RegionInputs::default(),
                now: now(),
            },
            |_settings, _region| ready(Err::<Credentials, _>(CredentialError::MissingCredentials)),
        ))
        .unwrap_err();
    for rendered in [format!("{error:?}"), format!("{error}")] {
        assert!(!rendered.contains(fixture_secret), "{rendered}");
    }
}
