use std::future::ready;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::files::records::FileKind;
use secrecy::ExposeSecret as _;

use super::super::S3ListingError;
use super::super::decode::{DecodeLimits, decode_s3_listing};
use super::super::requests::{
    S3FileMethod, S3SignedRequest, list_files_next_request, list_files_request,
    object_file_request, retrieve_file, s3_credentials, sign_without_body,
};
use super::super::responses::delete_file_response;
use super::super::target::s3_request_target;
use super::support::{
    EMPTY_SHA256, authorization, block_on, bucket_access, credentials, empty_environment, env_map,
    environment_credentials, fixed_now, header_pairs, header_value, list_access, listing_request,
    managed_files, null_purpose, params, raw_listing, region_inputs, region_params, text_params,
    token_credentials, unavailable_credentials,
};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::{S3AccessParams, TrustedValue, frozen_mapping};
use crate::signing::target::RegionInputs;

pub(super) fn file_retrieve_delete() {
    retrieve_is_unsupported();
    object_requests_require_a_file_id();
    content_request_is_signed();
    delete_request_is_signed();
    file_ids_are_decoded_into_keys();
    delete_rejections_carry_their_status();
    a_missing_bucket_is_reported();
}

fn token_params() -> Params {
    params(vec![
        (
            "aws_region_name",
            RequestValue::Text("us-east-1".to_owned()),
        ),
        (
            "aws_access_key_id",
            RequestValue::Text("AKIDEXAMPLE".to_owned()),
        ),
        (
            "aws_secret_access_key",
            RequestValue::Text("secret".to_owned()),
        ),
        (
            "aws_session_token",
            RequestValue::Text("tokEXAMPLE".to_owned()),
        ),
    ])
}

fn signed_object_request(
    litellm: &mut Params,
    access: &S3AccessParams,
    method: S3FileMethod,
    file_id: Option<&str>,
) -> Result<S3SignedRequest, S3ListingError> {
    let optional = region_params();
    let environment = empty_environment();
    block_on(object_file_request(
        listing_request(&optional, litellm, access, &environment),
        method,
        file_id,
        token_credentials,
    ))
}

fn retrieve_is_unsupported() {
    let error = retrieve_file().unwrap_err();
    assert!(matches!(error, S3ListingError::RetrievalUnsupported));
    assert_eq!(
        error.to_string(),
        "BedrockFilesConfig does not support file retrieval"
    );
}

fn object_requests_require_a_file_id() {
    let mut litellm = token_params();
    let access = bucket_access("in-bucket");
    let missing =
        signed_object_request(&mut litellm, &access, S3FileMethod::Content, None).unwrap_err();
    assert!(matches!(missing, S3ListingError::FileIdRequired));
    assert_eq!(
        missing.to_string(),
        "file_id is required for Bedrock file content retrieval"
    );
    let empty =
        signed_object_request(&mut litellm, &access, S3FileMethod::Content, Some("")).unwrap_err();
    assert!(matches!(empty, S3ListingError::FileIdRequired));
}

fn content_request_is_signed() {
    assert_object_request_signed(
        S3FileMethod::Content,
        "9911713a01b83beb7cfdc8b16ff8c86ed12eade33399b8d8107dfed6c27e128c",
    );
}

fn delete_request_is_signed() {
    assert_object_request_signed(
        S3FileMethod::Delete,
        "67f7305849bc15f5b462815692fa5eaedc6ae429f2a91ded713113f3e39ab960",
    );
}

fn assert_object_request_signed(method: S3FileMethod, signature: &str) {
    let mut litellm = token_params();
    let access = bucket_access("in-bucket");
    let signed = signed_object_request(
        &mut litellm,
        &access,
        method,
        Some("s3://in-bucket/litellm-batch-outputs/x.jsonl"),
    )
    .unwrap();
    assert_eq!(
        signed.url,
        "https://s3.us-east-1.amazonaws.com/in-bucket/litellm-batch-outputs%2Fx.jsonl"
    );
    assert!(signed.query.is_empty());
    assert_eq!(
        header_pairs(&litellm),
        vec![
            ("X-Amz-Date".to_owned(), "20240101T000000Z".to_owned()),
            ("X-Amz-Security-Token".to_owned(), "tokEXAMPLE".to_owned()),
            ("X-Amz-Content-SHA256".to_owned(), EMPTY_SHA256.to_owned()),
            (
                "Authorization".to_owned(),
                authorization(
                    "AKIDEXAMPLE",
                    "us-east-1",
                    "host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
                    signature
                )
            ),
        ]
    );
}

fn file_ids_are_decoded_into_keys() {
    let optional = region_params();
    let environment = empty_environment();
    let access = bucket_access("in-bucket");
    let mut litellm = token_params();
    let spaced = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Content,
        Some("s3://in-bucket/litellm-bedrock-files/a%2Fb%20c.jsonl"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(
        spaced.url,
        "https://s3.us-east-1.amazonaws.com/in-bucket/litellm-bedrock-files/a/b%20c.jsonl"
    );
}

fn delete_rejections_carry_their_status() {
    let mut litellm = token_params();
    let access = bucket_access("in-bucket");
    let missing_id =
        signed_object_request(&mut litellm, &access, S3FileMethod::Delete, None).unwrap_err();
    assert_eq!(missing_id.status_code(), Some(400));
    let unmanaged = signed_object_request(
        &mut litellm,
        &access,
        S3FileMethod::Delete,
        Some("s3://in-bucket/notes/readme.txt"),
    )
    .unwrap_err();
    assert_eq!(unmanaged.status_code(), Some(400));
}

fn a_missing_bucket_is_reported() {
    let optional = region_params();
    let environment = empty_environment();
    let mut litellm = token_params();
    let error = block_on(object_file_request(
        listing_request(
            &optional,
            &mut litellm,
            &S3AccessParams::default(),
            &environment,
        ),
        S3FileMethod::Content,
        Some("s3://in-bucket/litellm-bedrock-files-a"),
        unavailable_credentials,
    ))
    .unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("S3 bucket_name is required"));
}

pub(super) fn invalid_xml() {
    rejects_malformed_documents();
    reports_decode_failures_through_managed_files();
    reports_a_truncated_continuation_token();
}

fn rejects_malformed_documents() {
    assert!(matches!(
        decode_s3_listing(b"<a>", &DecodeLimits::DEFAULT),
        Err(S3ListingError::Unclosed)
    ));
    assert!(matches!(
        decode_s3_listing(b"", &DecodeLimits::DEFAULT),
        Err(S3ListingError::NoRoot)
    ));
    let mismatched = decode_s3_listing(b"<a></b>", &DecodeLimits::DEFAULT);
    assert!(matches!(mismatched, Err(S3ListingError::Xml(_))));
    assert!(matches!(
        decode_s3_listing(b"<a/><b/>", &DecodeLimits::DEFAULT),
        Err(S3ListingError::Trailing)
    ));
    assert!(matches!(
        decode_s3_listing(b"<a/><?xml version=\"1.0\"?>", &DecodeLimits::DEFAULT),
        Err(S3ListingError::Trailing)
    ));
    let truncated = b"<ListBucketResult><Contents><Key>litellm-bedrock-files-a</Key>";
    assert!(matches!(
        decode_s3_listing(truncated, &DecodeLimits::DEFAULT),
        Err(S3ListingError::Unclosed)
    ));
}

fn reports_decode_failures_through_managed_files() {
    let litellm = null_purpose();
    let unterminated = managed_files(b"<ListBucketResult>", &litellm).unwrap_err();
    assert_eq!(unterminated.status_code(), None);
    let empty = managed_files(b"", &litellm).unwrap_err();
    assert_eq!(empty.status_code(), None);
    let mismatched = managed_files(b"<a></b>", &litellm).unwrap_err();
    assert_eq!(mismatched.status_code(), None);
}

fn reports_a_truncated_continuation_token() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let mut litellm = region_params();
    let error = block_on(list_files_next_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        200,
        b"<ListBucketResult><NextContinuationToken>",
        environment_credentials,
    ))
    .unwrap_err();
    assert_eq!(error.status_code(), None);
}

pub(super) fn delete_response_context() {
    deletes_a_managed_file();
    requires_a_file_id();
    maps_failures_to_bedrock_errors();
}

fn deletes_a_managed_file() {
    let headers = vec![("x-amz-request-id".to_owned(), "abc".to_owned())];
    let success = raw_listing(204, b"", &headers);
    let args = text_params(&[("file_id", "s3://in-bucket/litellm-batch-outputs/x.jsonl")]);
    let deleted = delete_file_response(&success, Some(&args)).unwrap();
    assert_eq!(deleted.id, "s3://in-bucket/litellm-batch-outputs/x.jsonl");
    assert!(deleted.deleted);
    assert_eq!(deleted.object, FileKind::File);
}

fn requires_a_file_id() {
    let headers = vec![("x-amz-request-id".to_owned(), "abc".to_owned())];
    let success = raw_listing(204, b"", &headers);
    assert!(matches!(
        delete_file_response(&success, None),
        Err(S3ListingError::DeleteContext)
    ));
    let empty_id = text_params(&[("file_id", "")]);
    assert!(matches!(
        delete_file_response(&success, Some(&empty_id)),
        Err(S3ListingError::DeleteContext)
    ));
    let null_id = params(vec![("file_id", RequestValue::Raw("null".to_owned()))]);
    assert!(matches!(
        delete_file_response(&success, Some(&null_id)),
        Err(S3ListingError::DeleteContext)
    ));
}

fn maps_failures_to_bedrock_errors() {
    let headers = vec![("x-amz-request-id".to_owned(), "abc".to_owned())];
    let args = text_params(&[("file_id", "s3://in-bucket/litellm-batch-outputs/x.jsonl")]);

    let denied = raw_listing(500, b"boom", &headers);
    let denied_error = delete_file_response(&denied, Some(&args)).unwrap_err();
    assert_eq!(denied_error.status_code(), Some(500));
    assert_bedrock_reply(&denied_error, "boom", Some(headers.as_slice()));

    let odd = raw_listing(200, b"", &headers);
    let odd_error = delete_file_response(&odd, Some(&args)).unwrap_err();
    assert_eq!(odd_error.status_code(), Some(502));
    assert_bedrock_reply(
        &odd_error,
        "S3 file deletion returned HTTP 200",
        Some(headers.as_slice()),
    );

    let odd_body = raw_listing(200, b"bad", &headers);
    let body_error = delete_file_response(&odd_body, Some(&args)).unwrap_err();
    assert_eq!(body_error.status_code(), Some(502));
    assert_bedrock_reply(&body_error, "bad", None);

    let missing = raw_listing(404, b"", &headers);
    let missing_error = delete_file_response(&missing, Some(&args)).unwrap_err();
    assert_eq!(missing_error.status_code(), Some(404));
}

fn assert_bedrock_reply(
    error: &S3ListingError,
    message: &str,
    headers: Option<&[(String, String)]>,
) {
    let S3ListingError::Bedrock(inner) = error else {
        panic!("expected a Bedrock error, got {error:?}");
    };
    assert_eq!(inner.error.message, message);
    if let Some(expected) = headers {
        assert_eq!(inner.response_headers.as_slice(), expected);
    }
}

pub(super) fn s3_request_target_case() {
    explicit_s3_region_wins();
    aws_region_name_sets_the_region();
    optional_params_override_litellm_params();
    a_configured_endpoint_replaces_the_default();
    partition_suffixes_follow_the_region();
    an_empty_endpoint_falls_back_to_the_default();
    environment_regions_precede_the_default();
    a_session_region_precedes_the_default();
    the_default_region_is_us_west_2();
    invalid_regions_are_reported();
    static_key_pairs_sign_directly();
    aws_parameters_reach_the_credential_chain();
    missing_credentials_surface();
}

fn explicit_s3_region_wins() {
    let litellm = text_params(&[
        ("s3_region_name", "us-west-2"),
        ("aws_region_name", "eu-west-1"),
    ]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "us-west-2");
    assert_eq!(target.endpoint_url, "https://s3.us-west-2.amazonaws.com");
}

fn aws_region_name_sets_the_region() {
    let litellm = text_params(&[("aws_region_name", "eu-west-1")]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "eu-west-1");
    assert_eq!(target.endpoint_url, "https://s3.eu-west-1.amazonaws.com");
}

fn optional_params_override_litellm_params() {
    let optional = text_params(&[("s3_region_name", "ap-south-1")]);
    let litellm = text_params(&[("s3_region_name", "us-west-2")]);
    let target = s3_request_target(&optional, &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "ap-south-1");
    assert_eq!(target.endpoint_url, "https://s3.ap-south-1.amazonaws.com");
}

fn a_configured_endpoint_replaces_the_default() {
    let litellm = text_params(&[
        ("aws_region_name", "cn-north-1"),
        ("s3_endpoint_url", "https://example.com:9000/"),
    ]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "cn-north-1");
    assert_eq!(target.endpoint_url, "https://example.com:9000");
}

fn partition_suffixes_follow_the_region() {
    let litellm = text_params(&[("aws_region_name", "cn-north-1")]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(
        target.endpoint_url,
        "https://s3.cn-north-1.amazonaws.com.cn"
    );
}

fn an_empty_endpoint_falls_back_to_the_default() {
    let litellm = text_params(&[("aws_region_name", "us-east-1"), ("s3_endpoint_url", "")]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.endpoint_url, "https://s3.us-east-1.amazonaws.com");
}

fn environment_regions_precede_the_default() {
    let environment = env_map(&[
        ("AWS_REGION_NAME", "eu-central-1"),
        ("AWS_REGION", "ca-central-1"),
    ]);
    let target =
        s3_request_target(&Params::new(), &Params::new(), region_inputs(&environment)).unwrap();
    assert_eq!(target.aws_region_name, "eu-central-1");
}

fn a_session_region_precedes_the_default() {
    let inputs = RegionInputs {
        environment_region_name: None,
        environment_region: None,
        session_region: Some("ap-south-1"),
    };
    let target = s3_request_target(&Params::new(), &Params::new(), inputs).unwrap();
    assert_eq!(target.aws_region_name, "ap-south-1");
}

fn the_default_region_is_us_west_2() {
    let target =
        s3_request_target(&Params::new(), &Params::new(), RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "us-west-2");
    assert_eq!(target.endpoint_url, "https://s3.us-west-2.amazonaws.com");
}

fn invalid_regions_are_reported() {
    let upper = text_params(&[("aws_region_name", "US-EAST-1")]);
    let upper_error = s3_request_target(&Params::new(), &upper, RegionInputs::default())
        .err()
        .unwrap();
    assert_eq!(upper_error.status_code(), None);
    let empty = text_params(&[("aws_region_name", "")]);
    let empty_region = s3_request_target(&Params::new(), &empty, RegionInputs::default());
    assert_eq!(empty_region.err().unwrap().status_code(), None);
    let numeric = params(vec![(
        "aws_region_name",
        RequestValue::Raw("123".to_owned()),
    )]);
    let numeric_error = s3_request_target(&Params::new(), &numeric, RegionInputs::default())
        .err()
        .unwrap();
    assert!(
        matches!(
            &numeric_error,
            S3ListingError::Field { field, .. } if *field == "aws_region_name"
        ),
        "expected a Field error on aws_region_name, got {numeric_error:?}"
    );
}

fn static_key_pairs_sign_directly() {
    let litellm = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("aws_access_key_id", "STATICACCESS"),
        ("aws_secret_access_key", "STATICSECRET"),
        ("aws_session_token", "STATICTOKEN"),
        ("s3_access_key_id", "S3ACCESS"),
        ("s3_secret_access_key", "S3SECRET"),
    ]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    let resolved = block_on(s3_credentials(&target, unavailable_credentials)).unwrap();
    assert_eq!(resolved.access_key_id.expose_secret(), "S3ACCESS");
    assert_eq!(resolved.secret_access_key.expose_secret(), "S3SECRET");
    assert!(resolved.session_token.is_none());
}

fn aws_parameters_reach_the_credential_chain() {
    let litellm = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("aws_access_key_id", "STATICACCESS"),
        ("aws_secret_access_key", "STATICSECRET"),
        ("aws_session_token", "STATICTOKEN"),
    ]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    let mut seen_access = None;
    let mut seen_session = None;
    let mut seen_region = None;
    let resolved = block_on(s3_credentials(
        &target,
        |settings: &AwsSettings, region: &str| {
            seen_access = settings
                .access_key_id
                .as_ref()
                .map(|value| value.expose_secret().to_owned());
            seen_session = settings
                .session_token
                .as_ref()
                .map(|value| value.expose_secret().to_owned());
            seen_region = Some(region.to_owned());
            ready(Ok::<Credentials, CredentialError>(credentials(None)))
        },
    ))
    .unwrap();
    assert_eq!(seen_access.as_deref(), Some("STATICACCESS"));
    assert_eq!(seen_session.as_deref(), Some("STATICTOKEN"));
    assert_eq!(seen_region.as_deref(), Some("us-east-1"));
    assert_eq!(resolved.access_key_id.expose_secret(), "AKIDEXAMPLE");
    assert!(resolved.session_token.is_none());
}

fn missing_credentials_surface() {
    let target =
        s3_request_target(&Params::new(), &Params::new(), RegionInputs::default()).unwrap();
    let error = block_on(s3_credentials(&target, unavailable_credentials)).unwrap_err();
    assert!(matches!(
        error,
        S3ListingError::Credential(CredentialError::MissingCredentials)
    ));
}

pub(super) fn bodiless_s3_signature() {
    signs_the_default_listing();
    purpose_params_change_the_signature();
    a_cn_partition_signs_with_its_host();
    a_custom_endpoint_and_prefix_change_the_signature();
    custom_endpoint_headers_carry_the_session_token();
}

fn expected_signed_headers(
    region: &str,
    token: Option<&str>,
    signature: &str,
) -> Vec<(String, String)> {
    let mut headers = vec![("X-Amz-Date".to_owned(), "20240101T000000Z".to_owned())];
    let signed = token.map_or("host;x-amz-content-sha256;x-amz-date", |value| {
        headers.push(("X-Amz-Security-Token".to_owned(), value.to_owned()));
        "host;x-amz-content-sha256;x-amz-date;x-amz-security-token"
    });
    headers.push(("X-Amz-Content-SHA256".to_owned(), EMPTY_SHA256.to_owned()));
    headers.push((
        "Authorization".to_owned(),
        authorization("AKIDEXAMPLE", region, signed, signature),
    ));
    headers
}

fn assert_default_headers(litellm: &Params) {
    assert_eq!(
        header_pairs(litellm),
        expected_signed_headers(
            "us-east-1",
            None,
            "f2c6f153494cd7f15cc4726db61cfec89d62b1e441f98adc110b242901b1d978"
        )
    );
}

fn signs_the_default_listing() {
    let environment = empty_environment();
    let access = list_access();
    let optional = region_params();
    let mut litellm = region_params();
    let signed = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        None,
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://s3.us-east-1.amazonaws.com/bucket/");
    assert_eq!(
        signed.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "litellm-b".to_owned()),
        ]
    );
    assert_default_headers(&litellm);
}

fn purpose_params_change_the_signature() {
    let environment = empty_environment();
    let access = list_access();
    let optional = region_params();
    let mut litellm = region_params();
    block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        Some("other"),
        environment_credentials,
    ))
    .unwrap();
    assert_eq!(
        header_value(&litellm, "Authorization").unwrap(),
        authorization(
            "AKIDEXAMPLE",
            "us-east-1",
            "host;x-amz-content-sha256;x-amz-date",
            "346642ac3b733b94fea62ae70ecfdf91c70614d6bc0d917705db5f4733f723e7"
        )
    );
}

fn a_cn_partition_signs_with_its_host() {
    let cn = text_params(&[("aws_region_name", "cn-north-1")]);
    let cn_target = s3_request_target(&Params::new(), &cn, RegionInputs::default()).unwrap();
    let resolved = credentials(None);
    let cn_headers = sign_without_body(
        "GET",
        "https://s3.cn-north-1.amazonaws.com.cn/bucket/",
        &cn_target,
        &resolved,
        fixed_now(),
    )
    .unwrap();
    assert_eq!(
        cn_headers,
        vec![
            ("X-Amz-Date".to_owned(), "20240101T000000Z".to_owned()),
            ("X-Amz-Content-SHA256".to_owned(), EMPTY_SHA256.to_owned()),
            (
                "Authorization".to_owned(),
                authorization(
                    "AKIDEXAMPLE",
                    "cn-north-1",
                    "host;x-amz-content-sha256;x-amz-date",
                    "58d4bd82af73a14a4a16038f98c6e29e62dbbbd15034ab560fb7e92496f8edd4"
                )
            ),
        ]
    );
}

fn a_custom_endpoint_and_prefix_change_the_signature() {
    let environment = empty_environment();
    let custom_access = S3AccessParams {
        trusted: frozen_mapping([(
            "s3_bucket_name".to_owned(),
            TrustedValue::Text("bucket/p".to_owned()),
        )]),
        bucket_name: None,
        output_bucket_name: None,
    };
    let mut custom = text_params(&[
        ("aws_region_name", "eu-west-1"),
        ("s3_endpoint_url", "https://example.com:9000"),
        ("aws_session_token", "tokEXAMPLE"),
    ]);
    let signed = block_on(list_files_request(
        listing_request(&Params::new(), &mut custom, &custom_access, &environment),
        None,
        token_credentials,
    ))
    .unwrap();
    assert_eq!(signed.url, "https://example.com:9000/bucket/");
    assert_eq!(
        signed.query,
        vec![
            ("list-type".to_owned(), "2".to_owned()),
            ("prefix".to_owned(), "p/litellm-b".to_owned()),
        ]
    );
}

fn custom_endpoint_headers_carry_the_session_token() {
    let environment = empty_environment();
    let custom_access = S3AccessParams {
        trusted: frozen_mapping([(
            "s3_bucket_name".to_owned(),
            TrustedValue::Text("bucket/p".to_owned()),
        )]),
        bucket_name: None,
        output_bucket_name: None,
    };
    let mut custom = text_params(&[
        ("aws_region_name", "eu-west-1"),
        ("s3_endpoint_url", "https://example.com:9000"),
        ("aws_session_token", "tokEXAMPLE"),
    ]);
    block_on(list_files_request(
        listing_request(&Params::new(), &mut custom, &custom_access, &environment),
        None,
        token_credentials,
    ))
    .unwrap();
    assert_custom_endpoint_headers(&custom);
}

fn assert_custom_endpoint_headers(custom: &Params) {
    let headers = header_pairs(custom);
    assert_eq!(
        headers,
        expected_signed_headers(
            "eu-west-1",
            Some("tokEXAMPLE"),
            "3facd560b39c9c5cbc5b97e65256b253b542ceee696d6e5bf0770233519bc3f5"
        )
    );
    assert_eq!(
        headers.len(),
        4,
        "a bodiless request adds no content headers"
    );
}
