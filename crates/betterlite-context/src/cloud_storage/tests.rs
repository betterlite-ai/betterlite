#![cfg(test)]
use super::policy::{LegacyCloudSetting, TrustedCloudCredentials, allow_legacy};
use super::{
    CloudStorageError, Encoding, build_object_name, decode_cloud_object_reference,
    encode_object_name, is_managed_cloud_storage_uri, sanitize_component, sanitize_path,
    split_bucket_name, validate_cloud_object_path,
};
use std::fs;
#[rstest::rstest]
#[case::cloud_uri_detection(is_managed_cloud_storage_uri("s3://").to_string(), "true")]
#[case::object_component_sanitization(
    sanitize_component(Some("a\\\u{1c}hello world..\u{1c}"), "file"),
    "hello_world"
)]
#[case::object_path_sanitization(sanitize_path(Some("../a//b/.."), "file"), "a/b")]
#[case::configured_bucket_split(format!("{:?}", split_bucket_name(" bucket///prefix// ").unwrap()), "(\"bucket\", \"prefix\")")]
#[case::object_path_validation(format!("{:?}", validate_cloud_object_path("a//b")), "Err(Segment)")]
#[case::cloud_object_encoding(
    encode_object_name("a/%252e/%FF", Encoding::Gcs),
    "a%2F%252e%2F%EF%BF%BD"
)]
#[case::legacy_trusted_policy(allow_legacy(None).to_string(), "false")]
#[case::managed_id_authorization(format!("{:?}", decode_cloud_object_reference("gs://bucket/p/file", "gs://", "bucket/p", &["file"], false).unwrap()), "(\"bucket\", \"p/file\")")]
#[case::legacy_prefix_boundary(format!("{:?}", decode_cloud_object_reference("gs://bucket/prefix-evil/x", "gs://", "bucket/prefix", &[], true)), "Err(PrefixMismatch)")]
#[case::encoded_traversal_order(format!("{:?}", decode_cloud_object_reference("gs://bucket/%2e%2e/x", "gs://", "bucket", &[], true)), "Err(Segment)")]
#[case::long_name(sanitize_component(Some(&"x".repeat(300)), ""), &"x".repeat(255))]
#[case::empty_path(sanitize_path(Some("../__"), "fallback"), "fallback")]
#[case::missing_path(sanitize_path(None, ".."), "..")]
#[case::uppercase(is_managed_cloud_storage_uri("S3://bucket/x").to_string(), "false")]
#[case::https(is_managed_cloud_storage_uri("https://bucket/x").to_string(), "false")]
#[case::gcs(is_managed_cloud_storage_uri("gs://").to_string(), "true")]
#[case::uuid(
    build_object_name("raw/", Some("a/b"), "file", uuid::Uuid::nil()),
    "raw/00000000000000000000000000000000-b"
)]
#[case::s3_encoding(
    encode_object_name("a/%2F/%FF/~._-", Encoding::S3),
    "a///%EF%BF%BD/~._-"
)]
#[case::literal_path(format!("{:?}", validate_cloud_object_path("a\\b?#/%2e/")), "Ok(())")]
#[case::literal_bucket(format!("{:?}", split_bucket_name("bucket@host:port").unwrap()), "(\"bucket@host:port\", \"\")")]
fn cloud_storage_cases(#[case] actual: String, #[case] expected: &str) {
    assert_eq!(actual, expected);
}
#[rstest::rstest]
#[case::nonstring(None, "../fallback", "../fallback")]
#[case::empty(Some(".."), "fallback", "fallback")]
#[case::controls(Some("a\0\u{7f}b"), "", "a__b")]
#[case::runs(Some("a +\u{e9} b"), "", "a_b")]
#[case::trailing_slash(Some("x/"), "fallback", "fallback")]
#[case::no_decode(Some("a%20b"), "", "a_20b")]
fn sanitization(#[case] input: Option<&str>, #[case] fallback: &str, #[case] expected: &str) {
    assert_eq!(sanitize_component(input, fallback), expected);
}
#[rstest::rstest]
#[case::empty("", CloudStorageError::EmptyBucket)]
#[case::scheme("gs://bucket", CloudStorageError::BucketUri)]
#[case::query("bucket?x", CloudStorageError::BucketUri)]
#[case::fragment("bucket#x", CloudStorageError::BucketUri)]
#[case::control("bu\0cket", CloudStorageError::BucketControl)]
#[case::missing_bucket("/prefix", CloudStorageError::EmptyBucket)]
#[case::separator("bu\\cket", CloudStorageError::Separator)]
#[case::traversal("bucket/../x", CloudStorageError::Segment)]
fn bucket_failures(#[case] value: &str, #[case] expected: CloudStorageError) {
    assert_eq!(split_bucket_name(value).unwrap_err(), expected);
}
#[rstest::rstest]
#[case::empty("", CloudStorageError::EmptyObject)]
#[case::absolute("/x", CloudStorageError::Absolute)]
#[case::control("x\0", CloudStorageError::ObjectControl)]
#[case::dot("a/.", CloudStorageError::Segment)]
#[case::parent("a/../b", CloudStorageError::Segment)]
#[case::empty_parent("a//", CloudStorageError::Segment)]
fn path_failures(#[case] value: &str, #[case] expected: CloudStorageError) {
    assert_eq!(validate_cloud_object_path(value).unwrap_err(), expected);
}
#[rstest::rstest]
#[case::bucket_first("gs://bucket/../x", "bad?", &[], true, Err(CloudStorageError::BucketUri))]
#[case::unmanaged("gs://bucket/x", "bucket", &[], false, Err(CloudStorageError::Unmanaged))]
#[case::missing_object("gs://bucket", "bucket", &[], true, Err(CloudStorageError::MissingObject))]
#[case::scheme("s3://bucket/x", "bucket", &[], true, Err(CloudStorageError::Scheme("gs://".to_owned())))]
#[case::double_escape("gs://bucket/p/%252e", "bucket/p", &[], true, Ok("p/%2e"))]
#[case::vertex("gs://bucket/p/litellm-vertex-files/x", "bucket/p", &[super::VERTEX_AI_MANAGED_GCS_PREFIX], false, Ok("p/litellm-vertex-files/x"))]
#[case::batch("gs://bucket/litellm-bedrock-files-x", "bucket", &super::BEDROCK_MANAGED_S3_PREFIXES, false, Ok("litellm-bedrock-files-x"))]
#[case::upload("gs://bucket/litellm-bedrock-files/x", "bucket", &super::BEDROCK_MANAGED_S3_PREFIXES, false, Ok("litellm-bedrock-files/x"))]
#[case::output("gs://bucket/litellm-batch-outputs/x", "bucket", &super::BEDROCK_MANAGED_S3_PREFIXES, false, Ok("litellm-batch-outputs/x"))]
fn authorization_boundaries(
    #[case] uri: &str,
    #[case] bucket: &str,
    #[case] prefixes: &[&str],
    #[case] legacy: bool,
    #[case] expected: Result<&str, CloudStorageError>,
) {
    assert_eq!(
        decode_cloud_object_reference(uri, "gs://", bucket, prefixes, legacy)
            .map(|(_, object)| object),
        expected.map(str::to_owned)
    );
}
#[rstest::rstest]
#[case::yes(LegacyCloudSetting::Text("\u{1c} YeS ".to_owned()), true)]
#[case::on(LegacyCloudSetting::Text("on".to_owned()), true)]
#[case::one(LegacyCloudSetting::Text("1".to_owned()), true)]
#[case::true_text(LegacyCloudSetting::Text("true".to_owned()), true)]
#[case::no(LegacyCloudSetting::Text("0".to_owned()), false)]
#[case::boolean(LegacyCloudSetting::Boolean(true), true)]
#[case::false_boolean(LegacyCloudSetting::Boolean(false), false)]
#[case::other(LegacyCloudSetting::Other, false)]
fn trusted_policy(#[case] setting: LegacyCloudSetting, #[case] expected: bool) {
    assert_eq!(
        allow_legacy(Some(&TrustedCloudCredentials::from(setting))),
        expected
    );
}
#[test]
fn cloud_storage_credential_identity_isolation() {
    let enabled = TrustedCloudCredentials::from(LegacyCloudSetting::Boolean(true));
    let disabled = TrustedCloudCredentials::from(LegacyCloudSetting::Boolean(false));
    let uri = "gs://tenant-a/x";
    let yes = allow_legacy(Some(&enabled));
    let no = allow_legacy(Some(&disabled));
    decode_cloud_object_reference(uri, "gs://", "tenant-b", &[], yes).unwrap_err();
    decode_cloud_object_reference(uri, "gs://", "tenant-a", &[], no).unwrap_err();
    decode_cloud_object_reference(uri, "gs://", "tenant-a", &[], yes).unwrap();
}
#[test]
fn cloud_storage_secret_safe_failure() {
    let secret = "fixture-secret-credential";
    let error =
        decode_cloud_object_reference(&format!("gs://{secret}/x"), "gs://", "bucket", &[], true)
            .unwrap_err();
    assert!(!format!("{error:?} {error}").contains(secret));
    let credentials = TrustedCloudCredentials::from(LegacyCloudSetting::Text(secret.to_owned()));
    assert!(!format!("{credentials:?}").contains(secret));
}
#[test]
fn replays_the_cloud_object_path_corpus() {
    let files = fs::read_dir(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fuzz/corpus/cloud_object_path"
    ))
    .unwrap();
    let seeds: Vec<_> = files.map(|entry| entry.unwrap().path()).collect();
    assert!(seeds.len() >= 5);
    assert!(seeds.iter().all(|path| {
        let value = fs::read_to_string(path).unwrap();
        let result = decode_cloud_object_reference(&value, "gs://", "bucket", &[], true);
        result.is_ok()
            == (path.file_name().unwrap() == "valid" || path.file_name().unwrap() == "encoded")
    }));
}
