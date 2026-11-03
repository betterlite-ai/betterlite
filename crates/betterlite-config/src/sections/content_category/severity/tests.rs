#![cfg(test)]

#[test]
fn severity_order_is_low_medium_high() {
    super::tests::severity_thresholds_follow_low_medium_high_order();
}
