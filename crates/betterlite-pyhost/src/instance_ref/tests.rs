#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::{InstanceRef, RemoteModule, Storage};

#[rstest]
#[case::two_parts("a.b", "a", "b")]
#[case::nested("a.b.c", "a.b", "c")]
#[case::no_dot("lonely", "", "lonely")]
#[case::leading_dot(".leading", "", "leading")]
#[case::trailing_dot("trailing.", "trailing", "")]
#[case::empty("", "", "")]
fn splits_a_local_reference(#[case] value: &str, #[case] module: &str, #[case] instance: &str) {
    match InstanceRef::parse(value).unwrap() {
        InstanceRef::Local {
            module_name,
            instance_name,
        } => {
            assert_eq!(module_name, module);
            assert_eq!(instance_name, instance);
        }
        InstanceRef::Remote(remote) => panic!("expected a local reference, got {remote:?}"),
    }
}

type RemoteCase = (
    &'static str,
    Storage,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
);

#[rstest]
#[case::s3((
    "s3://bucket/loggers/custom_callbacks.proxy_handler_instance",
    Storage::S3,
    "bucket",
    "loggers/custom_callbacks",
    "proxy_handler_instance",
    "loggers/custom_callbacks.py",
))]
#[case::gcs((
    "gcs://bucket/loggers/custom_callbacks.proxy_handler_instance",
    Storage::Gcs,
    "bucket",
    "loggers/custom_callbacks",
    "proxy_handler_instance",
    "loggers/custom_callbacks.py",
))]
#[case::dots_in_the_module_path(("s3://bucket/a.b.c", Storage::S3, "bucket", "a.b", "c", "a.b.py"))]
#[case::empty_instance_name(("s3://bucket/module.", Storage::S3, "bucket", "module", "", "module.py"))]
fn parses_a_remote_reference(#[case] case: RemoteCase) {
    let (value, storage, bucket, module_path, instance_name, object_key) = case;
    match InstanceRef::parse(value).unwrap() {
        InstanceRef::Remote(remote) => {
            let expected = RemoteModule {
                storage,
                bucket: bucket.to_owned(),
                object_key: object_key.to_owned(),
                module_path: module_path.to_owned(),
                instance_name: instance_name.to_owned(),
            };
            assert_eq!(remote, expected);
        }
        InstanceRef::Local {
            module_name,
            instance_name: found_instance,
        } => panic!("expected a remote reference, got {module_name}.{found_instance}"),
    }
}

#[rstest]
#[case::s3_without_a_path(
    "s3://bucket-only",
    "Invalid URL format: s3://bucket-only. Expected: s3://bucket-name/path/to/module.instance"
)]
#[case::gcs_without_a_path(
    "gcs://bucket-only",
    "Invalid URL format: gcs://bucket-only. Expected: gcs://bucket-name/path/to/module.instance"
)]
#[case::py_extension(
    "s3://bucket/loggers/custom_callbacks.py",
    "Invalid URL format in s3://bucket/loggers/custom_callbacks.py. Don't include '.py' extension and you must specify the instance name. Expected format: s3://bucket/loggers/custom_callbacks.instance_name (e.g., s3://bucket/loggers/custom_callbacks.proxy_handler_instance)"
)]
#[case::gcs_py_extension(
    "gcs://bucket/mod.py",
    "Invalid URL format in gcs://bucket/mod.py. Don't include '.py' extension and you must specify the instance name. Expected format: gcs://bucket/mod.instance_name (e.g., gcs://bucket/mod.proxy_handler_instance)"
)]
#[case::no_instance_dot(
    "s3://bucket/loggers/custom_callbacks",
    "Invalid module specification in s3://bucket/loggers/custom_callbacks. Expected: path/to/module.instance_name"
)]
#[case::empty_path(
    "s3://bucket/",
    "Invalid module specification in s3://bucket/. Expected: path/to/module.instance_name"
)]
fn reports_a_remote_reference_error(#[case] value: &str, #[case] text: &str) {
    let error = InstanceRef::parse(value).unwrap_err();
    assert_eq!(error.to_string(), text);
}

#[rstest]
fn replays_the_instance_ref_corpus(#[files("fuzz/corpus/instance_ref/*")] path: PathBuf) {
    let text = fs::read_to_string(path).unwrap();
    drop(InstanceRef::parse(&text));
}
