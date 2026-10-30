use crate::label_sanitization::sanitize_prometheus_label_value;
#[test]
fn translates_and_escapes() {
    assert_eq!(
        sanitize_prometheus_label_value(Some("a\\b\"c\n\r\u{2028}\u{2029}")),
        Some("a\\\\b\\\"c    ".to_owned())
    );
    assert_eq!(sanitize_prometheus_label_value(None), None);
}
