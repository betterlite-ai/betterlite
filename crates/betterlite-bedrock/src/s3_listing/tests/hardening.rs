use std::fs;
use std::path::Path;

use super::super::S3ListingError;
use super::super::decode::{DecodeLimits, decode_s3_listing};
use super::super::requests::{S3FileMethod, list_files_request, object_file_request};
use super::support::{
    block_on, bucket_access, empty_environment, header_pairs, header_value, list_access,
    listing_request, params, region_params, text_params, unavailable_credentials,
};
use crate::request_policy::RequestValue;
use crate::s3_access::S3AccessParams;

#[test]
fn s3_listing_rejects_dtd_and_limits() {
    let doctype = b"<!DOCTYPE ListBucketResult><ListBucketResult><Name>x</Name></ListBucketResult>";
    assert!(matches!(
        decode_s3_listing(doctype, &DecodeLimits::DEFAULT),
        Err(S3ListingError::Dtd)
    ));
    let internal_entity = b"<!DOCTYPE ListBucketResult [<!ENTITY e \"v\">]><ListBucketResult><Name>&e;</Name></ListBucketResult>";
    assert!(matches!(
        decode_s3_listing(internal_entity, &DecodeLimits::DEFAULT),
        Err(S3ListingError::Dtd)
    ));
    let external_entity =
        b"<!DOCTYPE ListBucketResult [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><ListBucketResult/>";
    assert!(matches!(
        decode_s3_listing(external_entity, &DecodeLimits::DEFAULT),
        Err(S3ListingError::Dtd)
    ));
    let undefined = b"<ListBucketResult><Name>&missing;</Name></ListBucketResult>";
    assert!(matches!(
        decode_s3_listing(undefined, &DecodeLimits::DEFAULT),
        Err(S3ListingError::Entity { name }) if name == "missing"
    ));
    let predefined = b"<ListBucketResult><Name>a&amp;b</Name></ListBucketResult>";
    assert_eq!(
        decode_s3_listing(predefined, &DecodeLimits::DEFAULT)
            .unwrap()
            .name
            .as_deref(),
        Some("a&b")
    );

    let limits = DecodeLimits {
        body_bytes: 4,
        depth: 8,
    };
    assert!(matches!(
        decode_s3_listing(b"<root/>", &limits),
        Err(S3ListingError::BodyTooLarge { limit: 4 })
    ));
    let shallow = DecodeLimits {
        body_bytes: 1024,
        depth: 1,
    };
    assert!(matches!(
        decode_s3_listing(b"<a><b/></a>", &shallow),
        Err(S3ListingError::DepthExceeded { limit: 1 })
    ));
    let two = DecodeLimits {
        body_bytes: 1024,
        depth: 2,
    };
    decode_s3_listing(b"<a><b/></a>", &two).unwrap();
    let content = decode_s3_listing(
        b"<ListBucketResult><Name><b>a</b>b</Name><Contents><Key>k</Key></Contents></ListBucketResult>",
        &DecodeLimits::DEFAULT,
    )
    .unwrap();
    assert_eq!(content.name, None);
    assert_eq!(content.contents[0].key.as_deref(), Some("k"));
}

#[test]
fn replays_the_bedrock_services_s3_listing_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/bedrock_services_s3_listing");
    let mut total = 0_usize;
    for entry in fs::read_dir(&corpus).unwrap() {
        let path = entry.unwrap().path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .to_owned();
        let bytes = fs::read(&path).unwrap();
        let outcome = decode_s3_listing(&bytes, &DecodeLimits::DEFAULT);
        let valid = name.starts_with("valid") || name.starts_with("boundary-valid");
        assert_eq!(
            outcome.is_ok(),
            valid,
            "seed {name} decoded against its expected category"
        );
        total = total.saturating_add(1);
    }
    assert!(total > 0, "no bedrock_services_s3_listing fuzz seeds");
}

#[test]
fn bedrock_services_s3_listing_credential_identity_isolation() {
    let optional = region_params();
    let environment = empty_environment();
    let access = list_access();
    let first_keys = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("s3_access_key_id", "AKIDONE"),
        ("s3_secret_access_key", "SECRETONE"),
    ]);
    let second_keys = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("s3_access_key_id", "AKIDTWO"),
        ("s3_secret_access_key", "SECRETTWO"),
    ]);

    let mut litellm = first_keys.clone();
    let first = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap();
    assert_eq!(first.url, "https://s3.us-east-1.amazonaws.com/bucket/");
    let first_authorization = header_value(&litellm, "Authorization").unwrap();
    assert!(first_authorization.contains("Credential=AKIDONE/"));
    assert!(!first_authorization.contains("AKIDTWO"));
    assert!(!first_authorization.contains("SECRETTWO"));

    let mut litellm = second_keys;
    let second = block_on(list_files_request(
        listing_request(&optional, &mut litellm, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap();
    let second_authorization = header_value(&litellm, "Authorization").unwrap();
    assert!(second_authorization.contains("Credential=AKIDTWO/"));
    assert!(!second_authorization.contains("AKIDONE"));
    assert!(!second_authorization.contains("SECRETONE"));
    assert_ne!(first_authorization, second_authorization);
    assert_eq!(first.query, second.query);

    let mut reused = first_keys;
    block_on(list_files_request(
        listing_request(&optional, &mut reused, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap();
    block_on(list_files_request(
        listing_request(&optional, &mut reused, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap();
    assert_eq!(
        header_value(&reused, "Authorization").unwrap(),
        first_authorization
    );
    assert_eq!(header_pairs(&reused).len(), 3);
}

#[test]
fn bedrock_services_s3_listing_secret_safe_failure() {
    let optional = region_params();
    let environment = empty_environment();
    let access = bucket_access("in-bucket");

    let mut bad_region = text_params(&[
        ("aws_region_name", "NOT A REGION"),
        ("s3_access_key_id", "S3SECRETACCESS"),
        ("s3_secret_access_key", "S3SECRETVALUE"),
        ("aws_access_key_id", "AWSSECRETACCESS"),
        ("aws_secret_access_key", "AWSSECRETVALUE"),
        ("aws_session_token", "AWSSECRETTOKEN"),
    ]);
    let error = block_on(list_files_request(
        listing_request(&optional, &mut bad_region, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap_err();
    for rendered in [error.to_string(), format!("{error:?}")] {
        assert!(!rendered.contains("S3SECRETACCESS"));
        assert!(!rendered.contains("S3SECRETVALUE"));
        assert!(!rendered.contains("AWSSECRETACCESS"));
        assert!(!rendered.contains("AWSSECRETVALUE"));
        assert!(!rendered.contains("AWSSECRETTOKEN"));
    }

    let mut unavailable = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("aws_access_key_id", "AWSSECRETACCESS"),
        ("aws_secret_access_key", "AWSSECRETVALUE"),
        ("aws_session_token", "AWSSECRETTOKEN"),
        ("s3_access_key_id", ""),
        ("s3_secret_access_key", ""),
    ]);
    let error = block_on(list_files_request(
        listing_request(&optional, &mut unavailable, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap_err();
    for rendered in [error.to_string(), format!("{error:?}")] {
        assert!(!rendered.contains("AWSSECRETACCESS"));
        assert!(!rendered.contains("AWSSECRETVALUE"));
        assert!(!rendered.contains("AWSSECRETTOKEN"));
    }

    let mut bad_endpoint = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("aws_secret_access_key", "AWSSECRETVALUE"),
    ]);
    bad_endpoint.insert(
        "s3_endpoint_url".to_owned(),
        RequestValue::Raw("1".to_owned()),
    );
    let error = block_on(list_files_request(
        listing_request(&optional, &mut bad_endpoint, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap_err();
    assert_eq!(error.status_code(), None);
    assert!(!format!("{error:?}").contains("AWSSECRETVALUE"));

    let mut no_file_id = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("aws_access_key_id", "AWSSECRETACCESS"),
        ("aws_secret_access_key", "AWSSECRETVALUE"),
    ]);
    let error = block_on(object_file_request(
        listing_request(&optional, &mut no_file_id, &access, &environment),
        S3FileMethod::Content,
        Some(""),
        unavailable_credentials,
    ))
    .unwrap_err();
    let rendered = format!("{error:?}");
    assert!(!rendered.contains("AWSSECRETACCESS"));
    assert!(!rendered.contains("AWSSECRETVALUE"));

    let mut no_bucket = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("aws_access_key_id", "AWSSECRETACCESS"),
        ("aws_secret_access_key", "AWSSECRETVALUE"),
    ]);
    let error = block_on(list_files_request(
        listing_request(
            &optional,
            &mut no_bucket,
            &S3AccessParams::default(),
            &environment,
        ),
        None,
        unavailable_credentials,
    ))
    .unwrap_err();
    assert!(!format!("{error:?}").contains("AWSSECRETVALUE"));

    let mut static_pair = text_params(&[
        ("aws_region_name", "us-east-1"),
        ("s3_access_key_id", "S3SECRETACCESS"),
        ("s3_secret_access_key", "S3SECRETVALUE"),
    ]);
    let mut target_password = params(vec![(
        "aws_session_tags",
        RequestValue::Raw("null".to_owned()),
    )]);
    target_password.insert(
        "s3_secret_access_key".to_owned(),
        RequestValue::Secret(secrecy::SecretString::from("S3SECRETVALUE".to_owned())),
    );
    let error = block_on(list_files_request(
        listing_request(&optional, &mut static_pair, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap();
    assert!(error.query.len() >= 2);
    let secret_error = block_on(list_files_request(
        listing_request(&optional, &mut target_password, &access, &environment),
        None,
        unavailable_credentials,
    ))
    .unwrap_err();
    assert!(!format!("{secret_error:?}").contains("S3SECRETVALUE"));
}
