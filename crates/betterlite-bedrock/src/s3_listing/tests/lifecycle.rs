use std::future::ready;

use betterlite_config::sections::bedrock_settings::AwsSettings;
use betterlite_openai::files::records::FileKind;
use secrecy::ExposeSecret as _;

use super::super::decode::{DecodeLimits, decode_s3_listing};
use super::super::requests::{
    S3FileMethod, list_files_next_request, list_files_request, object_file_request, retrieve_file,
    s3_credentials, sign_without_body,
};
use super::super::responses::delete_file_response;
use super::super::target::s3_request_target;
use super::super::{LIST_FILES_PURPOSE_PARAM, S3ListingError};
use super::support::{
    EMPTY_SHA256, authorization, block_on, bucket_access, credentials, empty_environment, env_map,
    environment_credentials, fixed_now, header_pairs, header_value, list_access, listing_request,
    managed_files, params, raw_listing, region_params, text_params, token_credentials,
    unavailable_credentials,
};
use crate::aws_settings::sources::Credentials;
use crate::credentials::error::CredentialError;
use crate::request_policy::{Params, RequestValue};
use crate::s3_access::{S3AccessParams, TrustedValue, frozen_mapping};
use crate::signing::target::RegionInputs;

pub(super) fn file_retrieve_delete() {
    let error = retrieve_file().unwrap_err();
    assert!(matches!(error, S3ListingError::RetrievalUnsupported));
    assert_eq!(
        error.to_string(),
        "BedrockFilesConfig does not support file retrieval"
    );

    let optional = region_params();
    let environment = empty_environment();
    let access = bucket_access("in-bucket");
    let mut litellm = params(vec![
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
    ]);
    let error = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Content,
        None,
        token_credentials,
    ))
    .unwrap_err();
    assert!(matches!(error, S3ListingError::FileIdRequired));
    assert_eq!(
        error.to_string(),
        "file_id is required for Bedrock file content retrieval"
    );
    let error = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Content,
        Some(""),
        token_credentials,
    ))
    .unwrap_err();
    assert!(matches!(error, S3ListingError::FileIdRequired));

    let signed = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Content,
        Some("s3://in-bucket/litellm-batch-outputs/x.jsonl"),
        token_credentials,
    ))
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
                    "9911713a01b83beb7cfdc8b16ff8c86ed12eade33399b8d8107dfed6c27e128c"
                )
            ),
        ]
    );

    let signed = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Delete,
        Some("s3://in-bucket/litellm-batch-outputs/x.jsonl"),
        token_credentials,
    ))
    .unwrap();
    assert_eq!(
        signed.url,
        "https://s3.us-east-1.amazonaws.com/in-bucket/litellm-batch-outputs%2Fx.jsonl"
    );
    assert_eq!(
        header_value(&litellm, "Authorization").unwrap(),
        authorization(
            "AKIDEXAMPLE",
            "us-east-1",
            "host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
            "67f7305849bc15f5b462815692fa5eaedc6ae429f2a91ded713113f3e39ab960"
        )
    );

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

    let error = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Delete,
        None,
        token_credentials,
    ))
    .unwrap_err();
    assert_eq!(error.status_code(), Some(400));

    let error = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        S3FileMethod::Delete,
        Some("s3://in-bucket/notes/readme.txt"),
        token_credentials,
    ))
    .unwrap_err();
    assert_eq!(error.status_code(), Some(400));

    let no_bucket = S3AccessParams::default();
    let error = block_on(object_file_request(
        listing_request(&optional, &mut litellm, &no_bucket, &environment),
        S3FileMethod::Content,
        Some("s3://in-bucket/litellm-bedrock-files-a"),
        unavailable_credentials,
    ))
    .unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(error.to_string().contains("S3 bucket_name is required"));
}

pub(super) fn invalid_xml() {
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

    let litellm = params(vec![(
        LIST_FILES_PURPOSE_PARAM,
        RequestValue::Raw("null".to_owned()),
    )]);
    let error = managed_files(b"<ListBucketResult>", &litellm).unwrap_err();
    assert_eq!(error.status_code(), None);
    let error = managed_files(b"", &litellm).unwrap_err();
    assert_eq!(error.status_code(), None);
    let error = managed_files(b"<a></b>", &litellm).unwrap_err();
    assert_eq!(error.status_code(), None);

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
    let headers = vec![("x-amz-request-id".to_owned(), "abc".to_owned())];
    let success = raw_listing(204, b"", &headers);
    let args = text_params(&[("file_id", "s3://in-bucket/litellm-batch-outputs/x.jsonl")]);
    let deleted = delete_file_response(&success, Some(&args)).unwrap();
    assert_eq!(deleted.id, "s3://in-bucket/litellm-batch-outputs/x.jsonl");
    assert!(deleted.deleted);
    assert_eq!(deleted.object, FileKind::File);

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

    let denied = raw_listing(500, b"boom", &headers);
    let error = delete_file_response(&denied, Some(&args)).unwrap_err();
    assert_eq!(error.status_code(), Some(500));
    match &error {
        S3ListingError::Bedrock(inner) => {
            assert_eq!(inner.error.message, "boom");
            assert_eq!(inner.response_headers, headers);
        }
        other => panic!("expected a Bedrock error, got {other:?}"),
    }

    let odd = raw_listing(200, b"", &headers);
    let error = delete_file_response(&odd, Some(&args)).unwrap_err();
    assert_eq!(error.status_code(), Some(502));
    match &error {
        S3ListingError::Bedrock(inner) => {
            assert_eq!(inner.error.message, "S3 file deletion returned HTTP 200");
            assert_eq!(inner.response_headers, headers);
        }
        other => panic!("expected a Bedrock error, got {other:?}"),
    }

    let odd_body = raw_listing(200, b"bad", &headers);
    let error = delete_file_response(&odd_body, Some(&args)).unwrap_err();
    assert_eq!(error.status_code(), Some(502));
    match &error {
        S3ListingError::Bedrock(inner) => assert_eq!(inner.error.message, "bad"),
        other => panic!("expected a Bedrock error, got {other:?}"),
    }

    let missing = raw_listing(404, b"", &headers);
    let error = delete_file_response(&missing, Some(&args)).unwrap_err();
    assert_eq!(error.status_code(), Some(404));
}

pub(super) fn s3_request_target_case() {
    let litellm = text_params(&[
        ("s3_region_name", "us-west-2"),
        ("aws_region_name", "eu-west-1"),
    ]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "us-west-2");
    assert_eq!(target.endpoint_url, "https://s3.us-west-2.amazonaws.com");

    let litellm = text_params(&[("aws_region_name", "eu-west-1")]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "eu-west-1");
    assert_eq!(target.endpoint_url, "https://s3.eu-west-1.amazonaws.com");

    let optional = text_params(&[("s3_region_name", "ap-south-1")]);
    let litellm = text_params(&[("s3_region_name", "us-west-2")]);
    let target = s3_request_target(&optional, &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "ap-south-1");
    assert_eq!(target.endpoint_url, "https://s3.ap-south-1.amazonaws.com");

    let litellm = text_params(&[
        ("aws_region_name", "cn-north-1"),
        ("s3_endpoint_url", "https://example.com:9000/"),
    ]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "cn-north-1");
    assert_eq!(target.endpoint_url, "https://example.com:9000");

    let litellm = text_params(&[("aws_region_name", "cn-north-1")]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(
        target.endpoint_url,
        "https://s3.cn-north-1.amazonaws.com.cn"
    );

    let litellm = text_params(&[("aws_region_name", "us-east-1"), ("s3_endpoint_url", "")]);
    let target = s3_request_target(&Params::new(), &litellm, RegionInputs::default()).unwrap();
    assert_eq!(target.endpoint_url, "https://s3.us-east-1.amazonaws.com");

    let environment = env_map(&[
        ("AWS_REGION_NAME", "eu-central-1"),
        ("AWS_REGION", "ca-central-1"),
    ]);
    let target = s3_request_target(
        &Params::new(),
        &Params::new(),
        super::support::region_inputs(&environment),
    )
    .unwrap();
    assert_eq!(target.aws_region_name, "eu-central-1");

    let inputs = RegionInputs {
        environment_region_name: None,
        environment_region: None,
        session_region: Some("ap-south-1"),
    };
    let target = s3_request_target(&Params::new(), &Params::new(), inputs).unwrap();
    assert_eq!(target.aws_region_name, "ap-south-1");

    let target =
        s3_request_target(&Params::new(), &Params::new(), RegionInputs::default()).unwrap();
    assert_eq!(target.aws_region_name, "us-west-2");
    assert_eq!(target.endpoint_url, "https://s3.us-west-2.amazonaws.com");

    let litellm = text_params(&[("aws_region_name", "US-EAST-1")]);
    let error = s3_request_target(&Params::new(), &litellm, RegionInputs::default())
        .err()
        .unwrap();
    assert_eq!(error.status_code(), None);
    let litellm = text_params(&[("aws_region_name", "")]);
    let empty_region = s3_request_target(&Params::new(), &litellm, RegionInputs::default());
    assert_eq!(empty_region.err().unwrap().status_code(), None);

    let litellm = params(vec![(
        "aws_region_name",
        RequestValue::Raw("123".to_owned()),
    )]);
    let error = s3_request_target(&Params::new(), &litellm, RegionInputs::default())
        .err()
        .unwrap();
    match error {
        S3ListingError::Field { field, .. } => assert_eq!(field, "aws_region_name"),
        other => panic!("expected a Field error, got {other:?}"),
    }

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

    let target =
        s3_request_target(&Params::new(), &Params::new(), RegionInputs::default()).unwrap();
    let error = block_on(s3_credentials(&target, unavailable_credentials)).unwrap_err();
    assert!(matches!(
        error,
        S3ListingError::Credential(CredentialError::MissingCredentials)
    ));
}

pub(super) fn bodiless_s3_signature() {
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
    assert_eq!(
        header_pairs(&litellm),
        vec![
            ("X-Amz-Date".to_owned(), "20240101T000000Z".to_owned()),
            ("X-Amz-Content-SHA256".to_owned(), EMPTY_SHA256.to_owned()),
            (
                "Authorization".to_owned(),
                authorization(
                    "AKIDEXAMPLE",
                    "us-east-1",
                    "host;x-amz-content-sha256;x-amz-date",
                    "f2c6f153494cd7f15cc4726db61cfec89d62b1e441f98adc110b242901b1d978"
                )
            ),
        ]
    );

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
    assert_eq!(
        header_pairs(&custom),
        vec![
            ("X-Amz-Date".to_owned(), "20240101T000000Z".to_owned()),
            ("X-Amz-Security-Token".to_owned(), "tokEXAMPLE".to_owned()),
            ("X-Amz-Content-SHA256".to_owned(), EMPTY_SHA256.to_owned()),
            (
                "Authorization".to_owned(),
                authorization(
                    "AKIDEXAMPLE",
                    "eu-west-1",
                    "host;x-amz-content-sha256;x-amz-date;x-amz-security-token",
                    "3facd560b39c9c5cbc5b97e65256b253b542ceee696d6e5bf0770233519bc3f5"
                )
            ),
        ]
    );
    assert_eq!(
        header_pairs(&custom).len(),
        4,
        "a bodiless request adds no content headers"
    );
}
