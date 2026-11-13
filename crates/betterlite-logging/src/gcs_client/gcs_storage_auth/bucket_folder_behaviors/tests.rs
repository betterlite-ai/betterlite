#![cfg(test)]

use rstest::rstest;

use super::handle_folders_in_bucket_name;

#[test]
fn a_folder_path_moves_from_the_bucket_to_the_object_name() {
    let names = handle_folders_in_bucket_name("my-bucket/my-folder/dev", "my-object")
        .expect("the configured bucket is a name");
    assert_eq!(names.0, "my-bucket");
    assert_eq!(names.1, "my-folder/dev/my-object");
}

#[test]
fn a_bucket_without_a_folder_keeps_the_object_name() {
    let names = handle_folders_in_bucket_name("my-bucket", "my-object")
        .expect("the configured bucket is a name");
    assert_eq!(names.0, "my-bucket");
    assert_eq!(names.1, "my-object");
}

#[rstest]
#[case::one_trailing_slash("my-bucket/")]
#[case::two_trailing_slashes("my-bucket//")]
fn a_bucket_whose_folder_is_empty_keeps_the_object_name(#[case] bucket: &str) {
    let names = handle_folders_in_bucket_name(bucket, "my-object")
        .expect("the configured bucket is a name");
    assert_eq!(names.0, "my-bucket");
    assert_eq!(names.1, "my-object");
}

#[rstest]
#[case::empty("")]
#[case::blank("  ")]
#[case::uri_scheme("gs://my-bucket")]
#[case::query("my-bucket?generation=1")]
#[case::fragment("my-bucket#fragment")]
#[case::backslash("my\\bucket")]
#[case::control_character("my-bucket\u{7}")]
#[case::parent_segment("my-bucket/../escape")]
fn a_bucket_that_is_not_a_name_is_rejected(#[case] bucket: &str) {
    assert!(
        handle_folders_in_bucket_name(bucket, "my-object").is_err(),
        "{bucket:?} was accepted"
    );
}

#[test]
fn a_folder_with_repeated_slashes_is_trimmed_like_litellm() {
    let names = handle_folders_in_bucket_name("my-bucket//folder/", "my-object")
        .expect("the configured bucket is a name");
    assert_eq!(names.0, "my-bucket");
    assert_eq!(names.1, "folder/my-object");
}
