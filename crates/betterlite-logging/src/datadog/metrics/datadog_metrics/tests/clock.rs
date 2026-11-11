use rust_decimal::Decimal;

use crate::datadog::metrics::clock::{Clock as _, SystemClock};

// 2020-01-01T00:00:00Z, so the assertion stays true for every run after the epoch this crate targets
const YEAR_2020: i64 = 1_577_836_800;

#[test]
fn clock_reports_the_current_epoch_second() {
    let now = SystemClock.now_seconds();
    assert!(now > Decimal::from(YEAR_2020));
}
