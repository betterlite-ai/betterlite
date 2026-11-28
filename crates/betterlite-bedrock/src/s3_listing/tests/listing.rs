use betterlite_openai::files::records::{FileKind, FileStatus, FilesPurpose};
use rstest::rstest;

use super::super::decode::{DecodeLimits, decode_s3_listing, parse_created_at, parse_size};
use super::super::prefix::{
    BATCH_LISTING_PREFIX, BATCH_OUTPUT_LISTING_PREFIX, listing_bucket_name, listing_query,
    managed_listing_prefix, output_location_still_unlisted, setting_text, walked_listing_purpose,
};
use super::super::requests::{encode_query, list_files_next_request, list_files_request};
use super::super::responses::{S3ResponseContext, list_files_response};
use super::super::{LIST_FILES_LOCATION_PARAM, LIST_FILES_PURPOSE_PARAM, S3ListingError};
use super::support::{
    access_params, authorization, batch_params, block_on, bucket_access, bucket_pair_access,
    contents_entry, empty_environment, env_map, environment_credentials, header_value,
    input_output_access, list_access, listing_request, listing_xml, managed_files,
    null_location_params, null_purpose, params, raw_listing, region_params, stored_text,
};
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::{S3AccessParams, TrustedValue};

pub(super) fn list_purpose_prefix() {
    assert_eq!(
        managed_listing_prefix("", None).as_deref(),
        Some("litellm-b")
    );
    assert_eq!(
        managed_listing_prefix("", Some("")).as_deref(),
        Some("litellm-b")
    );
    assert_eq!(
        managed_listing_prefix("", Some("batch")).as_deref(),
        Some(BATCH_LISTING_PREFIX)
    );
    assert_eq!(
        managed_listing_prefix("", Some("batch_output")).as_deref(),
        Some(BATCH_OUTPUT_LISTING_PREFIX)
    );
    assert_eq!(managed_listing_prefix("", Some("other")), None);
    assert_eq!(
        managed_listing_prefix("base/prefix", Some("batch")).as_deref(),
        Some("base/prefix/litellm-bedrock-files")
    );
    listing_queries();
    setting_text_reads_strings();
    walked_purpose_prefers_the_location();
    an_empty_location_is_used_as_given();
    batch_purpose_reaches_the_listing();
    absent_purpose_stores_null();
    unknown_purpose_asks_for_no_keys();
}

fn listing_queries() {
    assert_eq!(
        listing_query("", None),
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "litellm-b".to_owned()),
        ]
    );
    assert_eq!(
        listing_query("", Some("other")),
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("max-keys".to_owned(), "0".to_owned()),
        ]
    );
}

fn setting_text_reads_strings() {
    let requested = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("batch".to_owned()),
    )]);
    assert_eq!(
        setting_text(&requested, LIST_FILES_PURPOSE_PARAM).as_deref(),
        Some("batch")
    );
    let missing = null_purpose();
    assert_eq!(setting_text(&missing, LIST_FILES_PURPOSE_PARAM), None);
    assert_eq!(setting_text(&Params::new(), LIST_FILES_PURPOSE_PARAM), None);
    let empty = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text(String::new()),
    )]);
    assert_eq!(
        setting_text(&empty, LIST_FILES_PURPOSE_PARAM).as_deref(),
        Some("")
    );
}

fn walked_purpose_prefers_the_location() {
    let walked = params(vec![
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Text("batch_output".to_owned()),
        ),
    ]);
    assert_eq!(
        walked_listing_purpose(&walked).as_deref(),
        Some("batch_output")
    );
    let null_location = params(vec![
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ]);
    assert_eq!(
        walked_listing_purpose(&null_location).as_deref(),
        Some("batch")
    );
}

fn an_empty_location_is_used_as_given() {
    let empty_location = params(vec![
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (LIST_FILES_LOCATION_PARAM, RequestValue::Text(String::new())),
    ]);
    assert_eq!(walked_listing_purpose(&empty_location).as_deref(), Some(""));
}

fn batch_purpose_reaches_the_listing() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("batch"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/bucket/");
    assert_eq!(
        signed.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "litellm-bedrock-files".to_owned()),
        ]
    );
    assert_eq!(
        stored_text(&litellm, LIST_FILES_PURPOSE_PARAM).as_deref(),
        Some("batch")
    );
    assert_eq!(
        stored_text(&litellm, LIST_FILES_LOCATION_PARAM).as_deref(),
        Some("batch")
    );
}

fn absent_purpose_stores_null() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        None,
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(
        signed.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "litellm-b".to_owned()),
        ]
    );
    assert!(matches!(
        litellm.get(LIST_FILES_PURPOSE_PARAM),
        Some(RequestValue::Raw(raw)) if raw == "null"
    ));
    assert!(matches!(
        litellm.get(LIST_FILES_LOCATION_PARAM),
        Some(RequestValue::Raw(raw)) if raw == "null"
    ));
    assert_eq!(walked_listing_purpose(&litellm), None);
}

fn unknown_purpose_asks_for_no_keys() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("other"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(
        signed.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("max-keys".to_owned(), "0".to_owned()),
        ]
    );
    assert_eq!(
        stored_text(&litellm, LIST_FILES_PURPOSE_PARAM).as_deref(),
        Some("other")
    );
}

pub(super) fn input_output_bucket_walk() {
    output_bucket_listing();
    batch_purpose_lists_the_input_bucket();
    environment_buckets_are_used();
    bucket_names_resolve_from_access_and_environment();
    output_location_unlisted_when_buckets_differ();
    next_request_uses_the_walked_location();
    a_batch_purpose_stops_the_walk();
    an_empty_purpose_is_stored_verbatim();
}

fn output_bucket_listing() {
    let optional = region_params();
    let environment = empty_environment();
    let access = input_output_access();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("batch_output"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/out-bucket/");
    assert_eq!(
        signed.query.get(1),
        Some(&("prefix".to_owned(), "litellm-batch-outputs/".to_owned()))
    );
}

fn batch_purpose_lists_the_input_bucket() {
    let optional = region_params();
    let environment = empty_environment();
    let access = input_output_access();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("batch"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/in-bucket/");
}

fn environment_buckets_are_used() {
    let optional = region_params();
    let environment = env_map(&[
        ("AWS_S3_BUCKET_NAME", "env-in"),
        ("AWS_S3_OUTPUT_BUCKET_NAME", "env-out"),
    ]);
    let access = S3AccessParams::default();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("batch_output"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/env-out/");
}

fn bucket_names_resolve_from_access_and_environment() {
    let environment = env_map(&[("AWS_S3_OUTPUT_BUCKET_NAME", "env-out")]);
    let access = bucket_access("in-bucket");
    assert_eq!(
        listing_bucket_name(&access, &environment, Some("batch_output")).unwrap(),
        "env-out"
    );
    assert_eq!(
        listing_bucket_name(&access, &environment, None).unwrap(),
        "in-bucket"
    );
    assert_eq!(
        listing_bucket_name(&access, &empty_environment(), Some("batch_output")).unwrap(),
        "in-bucket"
    );

    let output_only = access_params(&[(
        "s3_output_bucket_name",
        TrustedValue::Text("out-bucket".to_owned()),
    )]);
    assert_eq!(
        listing_bucket_name(&output_only, &empty_environment(), Some("batch_output")).unwrap(),
        "out-bucket"
    );
    let missing = listing_bucket_name(
        &S3AccessParams::default(),
        &empty_environment(),
        Some("batch_output"),
    );
    let error = missing.unwrap_err();
    assert!(error.to_string().contains("S3 bucket_name is required"));
}

fn output_location_unlisted_when_buckets_differ() {
    let different_prefix = bucket_pair_access();
    let blank = params(vec![
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ]);
    assert!(
        output_location_still_unlisted(&different_prefix, &empty_environment(), &blank).unwrap()
    );
    assert!(!output_location_still_unlisted(&list_access(), &empty_environment(), &blank).unwrap());
    let empty_purpose = params(vec![
        (LIST_FILES_PURPOSE_PARAM, RequestValue::Text(String::new())),
        (LIST_FILES_LOCATION_PARAM, RequestValue::Text(String::new())),
    ]);
    assert!(
        !output_location_still_unlisted(&different_prefix, &empty_environment(), &empty_purpose)
            .unwrap()
    );
}

fn next_request_uses_the_walked_location() {
    let optional = region_params();
    let different_prefix = bucket_pair_access();
    let mut litellm = null_location_params();
    let next = block_on(list_files_next_request(
        listing_request(
            &optional,
            &mut litellm,
            &different_prefix,
            &empty_environment(),
        ),
        200,
        b"<ListBucketResult></ListBucketResult>",
        environment_credentials,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(next.url, "https://s3.us-east-1.amazonaws.com/bucket/");
    assert_eq!(
        next.query.get(1),
        Some(&("prefix".to_owned(), "out/litellm-batch-outputs/".to_owned()))
    );
    assert_eq!(
        stored_text(&litellm, LIST_FILES_LOCATION_PARAM).as_deref(),
        Some("batch_output")
    );
}

fn a_batch_purpose_stops_the_walk() {
    let optional = region_params();
    let mut litellm = batch_params();
    let next = block_on(list_files_next_request(
        listing_request(
            &optional,
            &mut litellm,
            &bucket_pair_access(),
            &empty_environment(),
        ),
        200,
        b"<ListBucketResult></ListBucketResult>",
        environment_credentials,
    ))
    .unwrap();
    assert!(next.is_none());
}

fn an_empty_purpose_is_stored_verbatim() {
    let optional = region_params();
    let mut litellm = params(vec![
        (LIST_FILES_PURPOSE_PARAM, RequestValue::Text(String::new())),
        (LIST_FILES_LOCATION_PARAM, RequestValue::Text(String::new())),
    ]);
    let next = block_on(list_files_next_request(
        listing_request(
            &optional,
            &mut litellm,
            &bucket_pair_access(),
            &empty_environment(),
        ),
        200,
        b"<ListBucketResult></ListBucketResult>",
        environment_credentials,
    ))
    .unwrap();
    assert!(next.is_none());
    assert_eq!(
        stored_text(&litellm, LIST_FILES_LOCATION_PARAM).as_deref(),
        Some("")
    );
}

pub(super) fn pagination_continuation() {
    error_statuses_skip_pagination();
    continuation_tokens_extend_the_query();
    continuation_tokens_are_not_encoded();
    empty_tokens_stop_pagination();
    a_token_continues_the_walk();
    batch_listing_requests_are_signed();
    encoded_queries_match_python_urlencode();
}

// Python urlencode(..., quote_via=quote, safe="") joins the pairs with & and escapes / and spaces
fn encoded_queries_match_python_urlencode() {
    assert_eq!(
        encode_query(&[
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "p/litellm-b".to_owned()),
        ]),
        "list-type=2&prefix=p%2Flitellm-b"
    );
    assert_eq!(
        encode_query(&[("continuation-token".to_owned(), "a/b c".to_owned())]),
        "continuation-token=a%2Fb%20c"
    );
    assert!(encode_query(&[]).is_empty());
}

fn error_statuses_skip_pagination() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = batch_params();
    let next = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        500,
        b"not xml",
        environment_credentials,
    ))
    .unwrap();
    assert!(next.is_none());
}

fn continuation_tokens_extend_the_query() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let body = b"<ListBucketResult><NextContinuationToken>tok</NextContinuationToken><IsTruncated>false</IsTruncated></ListBucketResult>";
    let mut litellm = batch_params();
    let next = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        200,
        body,
        environment_credentials,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(next.url, "https://s3.us-east-1.amazonaws.com/bucket/");
    assert_eq!(
        next.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "litellm-bedrock-files".to_owned()),
            ("continuation-token".to_owned(), "tok".to_owned()),
        ]
    );
    assert_eq!(
        stored_text(&litellm, LIST_FILES_PURPOSE_PARAM).as_deref(),
        Some("batch")
    );
}

fn continuation_tokens_are_not_encoded() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let body = b"<ListBucketResult><NextContinuationToken>a/b c</NextContinuationToken></ListBucketResult>";
    let mut litellm = region_params();
    let next = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        200,
        body,
        environment_credentials,
    ))
    .unwrap()
    .unwrap();
    assert_eq!(
        next.query.get(2),
        Some(&("continuation-token".to_owned(), "a/b c".to_owned()))
    );
    assert_eq!(
        header_value(&litellm, "Authorization").unwrap(),
        authorization(
            "AKIDEXAMPLE",
            "us-east-1",
            "host;x-amz-content-sha256;x-amz-date",
            "54e3e2c7d78fef4bff4ddf6334da09a2fa3dcb7053b33fe5fa18eb103b113ce8"
        )
    );
}

fn empty_tokens_stop_pagination() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let body =
        b"<ListBucketResult><NextContinuationToken></NextContinuationToken></ListBucketResult>";
    let mut litellm = region_params();
    let next = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        200,
        body,
        environment_credentials,
    ))
    .unwrap();
    assert!(next.is_none());
}

fn a_token_continues_the_walk() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let body = b"<ListBucketResult><NextContinuationToken>tok</NextContinuationToken><IsTruncated>true</IsTruncated></ListBucketResult>";
    let mut litellm = region_params();
    let next = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        200,
        body,
        environment_credentials,
    ))
    .unwrap();
    assert!(next.is_some());
}

fn batch_listing_requests_are_signed() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("batch"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/bucket/");
    assert_eq!(
        signed.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "litellm-bedrock-files".to_owned()),
        ]
    );
}

pub(super) fn namespace_xml_list() {
    prefixed_documents_decode();
    the_default_namespace_applies();
    nested_contents_keep_the_outer_text();
    managed_files_from_a_namespaced_body();
    foreign_and_empty_bucket_names();
    a_denied_list_keeps_the_bedrock_reply();
    an_empty_body_is_not_a_root();
}

fn prefixed_documents_decode() {
    let prefixed = b"<s3:ListBucketResult xmlns:s3=\"http://s3.amazonaws.com/doc/2006-03-01/\"><s3:Name>in-bucket</s3:Name><s3:Contents><s3:Key>litellm-bedrock-files-a</s3:Key><s3:Size>7</s3:Size><s3:LastModified>2024-01-02T03:04:05Z</s3:LastModified></s3:Contents><s3:NextContinuationToken>tok</s3:NextContinuationToken></s3:ListBucketResult>";
    let listing = decode_s3_listing(prefixed, &DecodeLimits::DEFAULT).unwrap();
    assert_eq!(listing.name.as_deref(), Some("in-bucket"));
    assert_eq!(listing.next_continuation_token.as_deref(), Some("tok"));
    assert_eq!(listing.contents.len(), 1);
    assert_eq!(
        listing.contents[0].key.as_deref(),
        Some("litellm-bedrock-files-a")
    );
    assert_eq!(listing.contents[0].size.as_deref(), Some("7"));
}

fn the_default_namespace_applies() {
    let defaulted = b"<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>in-bucket</Name><Contents xmlns=\"urn:other\"><Key>litellm-bedrock-files-b</Key></Contents></ListBucketResult>";
    let listing = decode_s3_listing(defaulted, &DecodeLimits::DEFAULT).unwrap();
    assert_eq!(
        listing.contents[0].key.as_deref(),
        Some("litellm-bedrock-files-b")
    );
}

fn nested_contents_keep_the_outer_text() {
    let nested = b"<ListBucketResult><Contents><Contents><Key>litellm-bedrock-files-inner</Key></Contents><Key>litellm-bedrock-files-outer</Key></Contents><Wrapper><Contents><Key>litellm-bedrock-files-wrapper</Key></Contents></Wrapper></ListBucketResult>";
    let listing = decode_s3_listing(nested, &DecodeLimits::DEFAULT).unwrap();
    assert_eq!(listing.contents.len(), 1);
    assert_eq!(
        listing.contents[0].key.as_deref(),
        Some("litellm-bedrock-files-outer")
    );
}

fn managed_files_from_a_namespaced_body() {
    let body = listing_xml(&contents_entry(
        "litellm-bedrock-files-a",
        "1",
        "2024-01-02T03:04:05Z",
    ));
    let litellm = null_purpose();
    let files = managed_files(&body, &litellm).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "s3://in-bucket/litellm-bedrock-files-a");
}

fn foreign_and_empty_bucket_names() {
    let litellm = null_purpose();
    let foreign = b"<ListBucketResult><Name>other-bucket</Name><Contents><Key>litellm-bedrock-files-a</Key></Contents></ListBucketResult>";
    assert!(managed_files(foreign, &litellm).unwrap().is_empty());

    let empty_name = b"<ListBucketResult><Name></Name><Contents><Key>litellm-bedrock-files-a</Key></Contents></ListBucketResult>";
    let files = managed_files(empty_name, &litellm).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "s3://in-bucket/litellm-bedrock-files-a");
}

fn a_denied_list_keeps_the_bedrock_reply() {
    let litellm = null_purpose();
    let prefix_access = bucket_access("in-bucket/base");
    let environment = empty_environment();
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &prefix_access,
        environment: &environment,
    };
    let headers = vec![("x-amz-request-id".to_owned(), "abc".to_owned())];
    let denied = raw_listing(403, b"denied", &headers);
    let error = list_files_response(&denied, &context).unwrap_err();
    assert_eq!(error.status_code(), Some(403));
    let S3ListingError::Bedrock(reply) = &error else {
        panic!("expected a Bedrock error, got {error:?}");
    };
    assert_eq!(reply.error.status_code, 403);
    assert_eq!(reply.error.message, "denied");
    assert_eq!(reply.response_headers, headers);
    assert_eq!(reply.error.response.status_code, 403);
    assert_eq!(reply.error.response.request.method, "GET");
    assert_eq!(
        reply.error.response.request.url,
        "https://s3.us-east-1.amazonaws.com/in-bucket/?list-type=2"
    );
}

fn an_empty_body_is_not_a_root() {
    let litellm = null_purpose();
    let prefix_access = bucket_access("in-bucket/base");
    let environment = empty_environment();
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &prefix_access,
        environment: &environment,
    };
    let empty = raw_listing(204, b"", &[]);
    let error = list_files_response(&empty, &context).unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(
        matches!(error, S3ListingError::NoRoot),
        "expected NoRoot, got {error:?}"
    );
}

pub(super) fn unmanaged_key_filter() {
    lists_only_managed_keys();
    legacy_ids_are_listed_when_allowed();
    strict_ids_drop_unmanaged_keys();
    bucket_prefixes_are_stripped();
    requested_purposes_filter_the_entries();
}

fn lists_only_managed_keys() {
    let litellm = null_purpose();
    let mut contents = String::new();
    contents.push_str(&contents_entry("", "1", ""));
    contents.push_str("<Contents><Size>2</Size></Contents>");
    contents.push_str(&contents_entry("notes/readme.txt", "3", ""));
    contents.push_str(&contents_entry("litellm-bedrock-files-a", "4", ""));
    contents.push_str(&contents_entry(
        "litellm-bedrock-files/path/x.jsonl",
        "5",
        "",
    ));
    contents.push_str(&contents_entry("litellm-batch-outputs/y.jsonl", "6", ""));
    contents.push_str(&contents_entry("litellm-bedrock-files-%41bc", "7", ""));
    let body = listing_xml(&contents);
    let files = managed_files(&body, &litellm).unwrap();
    let ids: Vec<&str> = files.iter().map(|file| file.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "s3://in-bucket/litellm-bedrock-files-a",
            "s3://in-bucket/litellm-bedrock-files/path/x.jsonl",
            "s3://in-bucket/litellm-batch-outputs/y.jsonl",
            "s3://in-bucket/litellm-bedrock-files-%41bc",
        ]
    );
    assert_eq!(files[1].filename, "x.jsonl");
    assert_eq!(files[2].purpose, FilesPurpose::BatchOutput);
    assert_eq!(files[0].purpose, FilesPurpose::Batch);
    assert_eq!(files[3].filename, "litellm-bedrock-files-%41bc");
    assert_eq!(files[0].object, FileKind::File);
    assert_eq!(files[0].status, Some(FileStatus::Uploaded));
}

fn legacy_ids_are_listed_when_allowed() {
    let litellm = null_purpose();
    let legacy_access = access_params(&[
        ("s3_bucket_name", TrustedValue::Text("in-bucket".to_owned())),
        ("allow_legacy_cloud_file_ids", TrustedValue::Boolean(true)),
    ]);
    let environment = empty_environment();
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &legacy_access,
        environment: &environment,
    };
    let legacy = listing_xml(&contents_entry("custom/unmanaged.txt", "8", ""));
    let files = list_files_response(&raw_listing(200, &legacy, &[]), &context).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "s3://in-bucket/custom/unmanaged.txt");
    assert_eq!(files[0].purpose, FilesPurpose::Batch);
}

fn strict_ids_drop_unmanaged_keys() {
    let litellm = null_purpose();
    let strict_access = bucket_access("in-bucket");
    let environment = empty_environment();
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &strict_access,
        environment: &environment,
    };
    let legacy = listing_xml(&contents_entry("custom/unmanaged.txt", "8", ""));
    assert!(
        list_files_response(&raw_listing(200, &legacy, &[]), &context)
            .unwrap()
            .is_empty()
    );
}

fn bucket_prefixes_are_stripped() {
    let litellm = null_purpose();
    let prefix_access = bucket_access("in-bucket/base");
    let environment = empty_environment();
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &prefix_access,
        environment: &environment,
    };
    let prefixed = b"<ListBucketResult><Name>in-bucket</Name><Contents><Key>base/litellm-batch-outputs/out.jsonl</Key></Contents><Contents><Key>base/litellm-bedrock-files-x</Key></Contents><Contents><Key>litellm-batch-outputs/out</Key></Contents></ListBucketResult>";
    let files = list_files_response(&raw_listing(200, prefixed, &[]), &context).unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].purpose, FilesPurpose::BatchOutput);
    assert_eq!(files[1].purpose, FilesPurpose::Batch);
}

fn requested_purposes_filter_the_entries() {
    let both = listing_xml(&format!(
        "{}{}",
        contents_entry("litellm-bedrock-files-a", "1", ""),
        contents_entry("litellm-batch-outputs/b", "2", "")
    ));
    let requested_batch = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("batch".to_owned()),
    )]);
    let batch_files = managed_files(&both, &requested_batch).unwrap();
    assert_eq!(batch_files.len(), 1);
    assert_eq!(batch_files[0].purpose, FilesPurpose::Batch);
    let requested_output = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("batch_output".to_owned()),
    )]);
    let output_files = managed_files(&both, &requested_output).unwrap();
    assert_eq!(output_files.len(), 1);
    assert_eq!(output_files[0].purpose, FilesPurpose::BatchOutput);
    let requested_empty = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text(String::new()),
    )]);
    assert!(managed_files(&both, &requested_empty).unwrap().is_empty());
    let requested_unknown = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("weird".to_owned()),
    )]);
    assert!(managed_files(&both, &requested_unknown).unwrap().is_empty());
}

pub(super) fn last_modified_and_size() {
    parses_sizes();
    parses_timestamps();
    rejects_bad_sizes();
    rejects_bad_timestamps("2024-01-02");
    offsets_and_pre_epoch_times();
    managed_files_carry_size_and_time();
    bad_size_and_bad_date_fail();
}

fn parses_sizes() {
    assert_eq!(parse_size(None).unwrap(), 0);
    assert_eq!(parse_size(Some("")).unwrap(), 0);
    assert_eq!(parse_size(Some("0")).unwrap(), 0);
    assert_eq!(parse_size(Some("+1_0")).unwrap(), 10);
    assert_eq!(parse_size(Some(" 42 ")).unwrap(), 42);
    assert_eq!(parse_size(Some("-5")).unwrap(), -5);
    assert_eq!(parse_size(Some("-0")).unwrap(), 0);
}

fn rejects_bad_sizes() {
    assert!(
        matches!(parse_size(Some("1__0")), Err(S3ListingError::Size { text }) if text == "1__0")
    );
    assert!(matches!(
        parse_size(Some("1_")),
        Err(S3ListingError::Size { .. })
    ));
    assert!(matches!(
        parse_size(Some("_1")),
        Err(S3ListingError::Size { .. })
    ));
    assert!(matches!(
        parse_size(Some("12a")),
        Err(S3ListingError::Size { .. })
    ));
    assert!(matches!(
        parse_size(Some(" ")),
        Err(S3ListingError::Size { .. })
    ));
    assert!(matches!(
        parse_size(Some("9223372036854775808")),
        Err(S3ListingError::Size { .. })
    ));
}

fn parses_timestamps() {
    let base = "2024-01-02T03:04:05";
    let expected = 1_704_164_645;
    assert_eq!(parse_created_at(None).unwrap(), 0);
    assert_eq!(parse_created_at(Some("")).unwrap(), 0);
    assert_eq!(parse_created_at(Some(base)).unwrap(), expected);
    assert_eq!(
        parse_created_at(Some(format!("{base}Z").as_str())).unwrap(),
        expected
    );
    assert_eq!(
        parse_created_at(Some(format!("{base}.500Z").as_str())).unwrap(),
        expected
    );
    assert_eq!(
        parse_created_at(Some(format!("{base}.1234567Z").as_str())).unwrap(),
        expected
    );
    assert_eq!(
        parse_created_at(Some(format!("{base},5Z").as_str())).unwrap(),
        expected
    );
}

#[rstest]
#[case::basic_format("20240102T030405Z")]
#[case::date_without_a_time("2024-01-02")]
#[case::compact_offset("2024-01-02T03:04:05+0200")]
#[case::hour_offset("2024-01-02T03:04:05+02")]
#[case::zero_year("0000-01-01T00:00:00Z")]
#[case::not_a_date("not-a-date")]
#[case::lowercase_z("2024-01-02T03:04:05z")]
#[case::february_30("2024-02-30T00:00:00Z")]
#[case::hour_24("2024-01-02T24:00:00Z")]
#[case::offset_24("2024-01-02T03:04:05+24:00")]
fn rejects_bad_timestamps(#[case] text: &str) {
    assert!(matches!(
        parse_created_at(Some(text)),
        Err(S3ListingError::Timestamp { .. })
    ));
}

fn offsets_and_pre_epoch_times() {
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05+02:00")).unwrap(),
        1_704_157_445
    );
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05-05:30")).unwrap(),
        1_704_184_445
    );
    assert_eq!(
        parse_created_at(Some("1969-12-31T23:59:59.500Z")).unwrap(),
        0
    );
    assert_eq!(
        parse_created_at(Some("1970-01-01T00:30:00+01:00")).unwrap(),
        -1800
    );
    assert_eq!(
        parse_created_at(Some("1970-01-01T00:30:00.500+01:00")).unwrap(),
        -1799
    );
    assert_eq!(
        parse_created_at(Some("1970-01-01T00:00:00.500Z")).unwrap(),
        0
    );
}

fn managed_files_carry_size_and_time() {
    let litellm = null_purpose();
    let body = listing_xml(&format!(
        "{}{}{}",
        contents_entry(
            "litellm-bedrock-files-a",
            "+1_0",
            "2024-01-02T03:04:05.500Z"
        ),
        "<Contents><Key>litellm-bedrock-files-b</Key></Contents>",
        contents_entry("litellm-bedrock-files/dir/", "3", "2024-01-02T03:04:05Z")
    ));
    let files = managed_files(&body, &litellm).unwrap();
    assert_eq!(files[0].bytes, 10);
    assert_eq!(files[0].created_at, 1_704_164_645);
    assert_eq!(files[0].filename, "litellm-bedrock-files-a");
    assert_eq!(files[1].bytes, 0);
    assert_eq!(files[1].created_at, 0);
    assert_eq!(files[2].filename, "");
}

fn bad_size_and_bad_date_fail() {
    let litellm = null_purpose();
    let bad_size = listing_xml(&contents_entry("litellm-bedrock-files-a", "1__0", ""));
    let size_error = managed_files(&bad_size, &litellm).unwrap_err();
    assert_eq!(size_error.status_code(), None);
    assert!(matches!(size_error, S3ListingError::Size { .. }));

    let bad_date = listing_xml(&contents_entry("litellm-bedrock-files-a", "1", "bad"));
    let date_error = managed_files(&bad_date, &litellm).unwrap_err();
    assert_eq!(date_error.status_code(), None);
    assert!(matches!(date_error, S3ListingError::Timestamp { .. }));
}
