use crate::label_sanitization::sanitize_prometheus_label_name;
#[test]
fn replaces_invalid() {
    assert_eq!(sanitize_prometheus_label_name("9bad-name"), "_9bad_name");
    assert_eq!(sanitize_prometheus_label_name(""), "_");
}
