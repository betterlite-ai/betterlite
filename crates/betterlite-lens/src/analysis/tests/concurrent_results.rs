#![cfg(test)]

#[test]
fn default_concurrency() {
    assert_eq!(super::ANALYSIS_CONCURRENCY, 8);
}
