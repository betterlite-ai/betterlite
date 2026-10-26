#![cfg(test)]
#[test]
fn depth_limit_is_bounded() {
    assert_eq!(super::MAX_STRUCTURED_CONTENT_SCAN_DEPTH, 20);
}
