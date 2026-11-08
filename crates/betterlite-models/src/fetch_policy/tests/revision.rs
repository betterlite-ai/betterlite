use crate::fetch_policy::revision::git_blob_id;

#[test]
fn catalog_fetch_policy_behavior_tests_behavior() {
    assert_eq!(git_blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    assert_eq!(
        git_blob_id(b"{}\n"),
        "0967ef424bce6791893e9a57bb952f80fd536e93"
    );
    assert_eq!(
        git_blob_id(br#"{"model":{}}"#),
        "fb7f2f10feada12cdd2b8cda7d85084fb1a7aa50"
    );
    assert_eq!(
        git_blob_id("h\u{e9}llo\n".as_bytes()),
        "5fb50d3c93474f139362304b663fe44e9d17a26e"
    );
}
