use betterlite_openai::files::records::{FileKind, FileStatus, FilesPurpose};

use super::super::decode::{DecodeLimits, decode_s3_listing, parse_created_at, parse_size};
use super::super::prefix::{
    BATCH_LISTING_PREFIX, BATCH_OUTPUT_LISTING_PREFIX, listing_bucket_name, listing_query,
    managed_listing_prefix, output_location_still_unlisted, setting_text, walked_listing_purpose,
};
use super::super::requests::{list_files_next_request, list_files_request};
use super::super::responses::{S3ResponseContext, list_files_response};
use super::super::{LIST_FILES_LOCATION_PARAM, LIST_FILES_PURPOSE_PARAM, S3ListingError};
use super::support::{
    access, authorization, block_on, bucket_access, contents_entry, empty_environment, env_map,
    environment_credentials, header_value, list_access, listing_request, listing_xml,
    managed_files, params, raw_listing, region_params, stored_text,
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

    let requested = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("batch".to_owned()),
    )]);
    assert_eq!(
        setting_text(&requested, LIST_FILES_PURPOSE_PARAM).as_deref(),
        Some("batch")
    );
    let missing = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Raw("null".to_owned()),
    )]);
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
    let empty_location = params(vec![
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (LIST_FILES_LOCATION_PARAM, RequestValue::Text(String::new())),
    ]);
    assert_eq!(walked_listing_purpose(&empty_location).as_deref(), Some(""));

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
    let optional = region_params();
    let environment = empty_environment();
    let access = access(&[
        ("s3_bucket_name", TrustedValue::Text("in-bucket".to_owned())),
        (
            "s3_output_bucket_name",
            TrustedValue::Text("out-bucket".to_owned()),
        ),
    ]);
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

    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("batch"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/in-bucket/");

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

    let access = access(&[(
        "s3_output_bucket_name",
        TrustedValue::Text("out-bucket".to_owned()),
    )]);
    assert_eq!(
        listing_bucket_name(&access, &empty_environment(), Some("batch_output")).unwrap(),
        "out-bucket"
    );
    let missing = listing_bucket_name(
        &S3AccessParams::default(),
        &empty_environment(),
        Some("batch_output"),
    );
    let error = missing.unwrap_err();
    assert!(error.to_string().contains("S3 bucket_name is required"));

    let different_prefix = access(&[
        ("s3_bucket_name", TrustedValue::Text("bucket/in".to_owned())),
        (
            "s3_output_bucket_name",
            TrustedValue::Text("bucket/out".to_owned()),
        ),
    ]);
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

    let mut litellm = params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ]);
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
        Some(&("prefix".to_owned(), "litellm-batch-outputs/".to_owned()))
    );
    assert_eq!(
        stored_text(&litellm, LIST_FILES_LOCATION_PARAM).as_deref(),
        Some("batch_output")
    );

    let mut litellm = params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ]);
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
    .unwrap();
    assert!(next.is_none());

    let mut litellm = empty_purpose;
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
    .unwrap();
    assert!(next.is_none());
    assert_eq!(
        stored_text(&litellm, LIST_FILES_LOCATION_PARAM).as_deref(),
        Some("")
    );
}

pub(super) fn pagination_continuation() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ]);
    let next = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        500,
        b"not xml",
        environment_credentials,
    ))
    .unwrap();
    assert!(next.is_none());

    let body = b"<ListBucketResult><NextContinuationToken>tok</NextContinuationToken><IsTruncated>false</IsTruncated></ListBucketResult>";
    let mut litellm = params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            LIST_FILES_PURPOSE_PARAM,
            RequestValue::Text("batch".to_owned()),
        ),
        (
            LIST_FILES_LOCATION_PARAM,
            RequestValue::Raw("null".to_owned()),
        ),
    ]);
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

    let defaulted = b"<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>in-bucket</Name><Contents xmlns=\"urn:other\"><Key>litellm-bedrock-files-b</Key></Contents></ListBucketResult>";
    let listing = decode_s3_listing(defaulted, &DecodeLimits::DEFAULT).unwrap();
    assert_eq!(
        listing.contents[0].key.as_deref(),
        Some("litellm-bedrock-files-b")
    );

    let nested = b"<ListBucketResult><Contents><Contents><Key>litellm-bedrock-files-inner</Key></Contents><Key>litellm-bedrock-files-outer</Key></Contents><Wrapper><Contents><Key>litellm-bedrock-files-wrapper</Key></Contents></Wrapper></ListBucketResult>";
    let listing = decode_s3_listing(nested, &DecodeLimits::DEFAULT).unwrap();
    assert_eq!(listing.contents.len(), 1);
    assert_eq!(
        listing.contents[0].key.as_deref(),
        Some("litellm-bedrock-files-outer")
    );

    let body = listing_xml(&contents_entry(
        "litellm-bedrock-files-a",
        "1",
        "2024-01-02T03:04:05Z",
    ));
    let litellm = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Raw("null".to_owned()),
    )]);
    let files = managed_files(&body, &litellm).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "s3://in-bucket/litellm-bedrock-files-a");

    let foreign = b"<ListBucketResult><Name>other-bucket</Name><Contents><Key>litellm-bedrock-files-a</Key></Contents></ListBucketResult>";
    assert!(managed_files(foreign, &litellm).unwrap().is_empty());

    let empty_name = b"<ListBucketResult><Name></Name><Contents><Key>litellm-bedrock-files-a</Key></Contents></ListBucketResult>";
    let files = managed_files(empty_name, &litellm).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, "s3://in-bucket/litellm-bedrock-files-a");

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
    match &error {
        S3ListingError::Bedrock(inner) => {
            assert_eq!(inner.error.status_code, 403);
            assert_eq!(inner.error.message, "denied");
            assert_eq!(inner.response_headers, headers);
            assert!(inner.error.response.is_some());
        }
        other => panic!("expected a Bedrock error, got {other:?}"),
    }

    let empty = raw_listing(204, b"", &[]);
    let error = list_files_response(&empty, &context).unwrap_err();
    assert_eq!(error.status_code(), None);
    match error {
        S3ListingError::NoRoot => {}
        other => panic!("expected NoRoot, got {other:?}"),
    }
}

pub(super) fn unmanaged_key_filter() {
    let litellm = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Raw("null".to_owned()),
    )]);
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

    let legacy_access = access(&[
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

    let strict_access = bucket_access("in-bucket");
    let context = S3ResponseContext {
        litellm_params: &litellm,
        access: &strict_access,
        environment: &environment,
    };
    assert!(
        list_files_response(&raw_listing(200, &legacy, &[]), &context)
            .unwrap()
            .is_empty()
    );

    let prefix_access = bucket_access("in-bucket/base");
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

    let both = listing_xml(&format!(
        "{}{}",
        contents_entry("litellm-bedrock-files-a", "1", ""),
        contents_entry("litellm-batch-outputs/b", "2", "")
    ));
    let requested_batch = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("batch".to_owned()),
    )]);
    let files = managed_files(&both, &requested_batch).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].purpose, FilesPurpose::Batch);
    let requested_output = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Text("batch_output".to_owned()),
    )]);
    let files = managed_files(&both, &requested_output).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].purpose, FilesPurpose::BatchOutput);
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
    assert_eq!(parse_size(None).unwrap(), 0);
    assert_eq!(parse_size(Some("")).unwrap(), 0);
    assert_eq!(parse_size(Some("0")).unwrap(), 0);
    assert_eq!(parse_size(Some("+1_0")).unwrap(), 10);
    assert_eq!(parse_size(Some(" 42 ")).unwrap(), 42);
    assert_eq!(parse_size(Some("-5")).unwrap(), -5);
    assert_eq!(parse_size(Some("-0")).unwrap(), 0);
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

    assert_eq!(parse_created_at(None).unwrap(), 0);
    assert_eq!(parse_created_at(Some("")).unwrap(), 0);
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05Z")).unwrap(),
        1_704_164_645
    );
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05.500Z")).unwrap(),
        1_704_164_645
    );
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05.1234567Z")).unwrap(),
        1_704_164_645
    );
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05,5Z")).unwrap(),
        1_704_164_645
    );
    assert_eq!(
        parse_created_at(Some("2024-01-02T03:04:05")).unwrap(),
        1_704_164_645
    );
    for unsupported in [
        "20240102T030405Z",
        "2024-01-02",
        "2024-01-02T03:04:05+0200",
        "2024-01-02T03:04:05+02",
    ] {
        assert!(
            matches!(
                parse_created_at(Some(unsupported)),
                Err(S3ListingError::Timestamp { .. })
            ),
            "{unsupported}"
        );
    }
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
    assert!(matches!(
        parse_created_at(Some("0000-01-01T00:00:00Z")),
        Err(S3ListingError::Timestamp { .. })
    ));
    assert!(matches!(
        parse_created_at(Some("not-a-date")),
        Err(S3ListingError::Timestamp { .. })
    ));
    assert!(matches!(
        parse_created_at(Some("2024-01-02T03:04:05z")),
        Err(S3ListingError::Timestamp { .. })
    ));
    assert!(matches!(
        parse_created_at(Some("2024-02-30T00:00:00Z")),
        Err(S3ListingError::Timestamp { .. })
    ));
    assert!(matches!(
        parse_created_at(Some("2024-01-02T24:00:00Z")),
        Err(S3ListingError::Timestamp { .. })
    ));
    assert!(matches!(
        parse_created_at(Some("2024-01-02T03:04:05+24:00")),
        Err(S3ListingError::Timestamp { .. })
    ));

    let litellm = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Raw("null".to_owned()),
    )]);
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

    let bad_size = listing_xml(&contents_entry("litellm-bedrock-files-a", "1__0", ""));
    let error = managed_files(&bad_size, &litellm).unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(matches!(error, S3ListingError::Size { .. }));

    let bad_date = listing_xml(&contents_entry("litellm-bedrock-files-a", "1", "bad"));
    let error = managed_files(&bad_date, &litellm).unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(matches!(error, S3ListingError::Timestamp { .. }));
}
