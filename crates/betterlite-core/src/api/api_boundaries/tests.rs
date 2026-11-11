#![cfg(test)]

use rstest::rstest;

use super::{
    ApiBoundariesInput, ApiBoundariesResult, ApiLimitError, ApiLimits, EventBoundary,
    ItemsBoundary, JsonBoundary, LimitErrorFields,
};

// the caps are literals here (0x0001_0000 is 65536), so a mutated bound constant cannot move the test with it
const JSON_CAP: usize = 16 * 1024 * 1024;
const EVENT_CAP: usize = 1024 * 1024;
const ITEMS_CAP: usize = 0x0001_0000;

#[rstest]
#[case::empty(0)]
#[case::one_byte(1)]
#[case::one_below_cap(JSON_CAP - 1)]
#[case::at_cap(JSON_CAP)]
fn json_at_cap(#[case] known_bytes: usize) {
    let limits = ApiLimits::default();
    let checked = limits
        .check_json(JsonBoundary { known_bytes })
        .expect("JSON at or below the cap is accepted");
    assert_eq!(
        checked,
        ApiBoundariesResult {
            observed: known_bytes,
            limit: JSON_CAP
        }
    );
}

#[rstest]
#[case::cap_plus_one(JSON_CAP + 1)]
#[case::twice_the_cap(JSON_CAP * 2)]
#[case::usize_max(usize::MAX)]
fn json_over_cap(#[case] known_bytes: usize) {
    let limits = ApiLimits::default();
    let error = limits
        .check_json(JsonBoundary { known_bytes })
        .expect_err("JSON over the cap is rejected");
    assert_eq!(
        error,
        ApiLimitError::JsonTooLarge {
            observed: known_bytes,
            limit: JSON_CAP,
        }
    );
    assert_eq!(
        error.fields(),
        LimitErrorFields {
            status: 413,
            error_type: "invalid_request_error",
            param: "body",
            code: "request_too_large",
        }
    );
}

#[rstest]
#[case::empty(0)]
#[case::one_byte(1)]
#[case::one_below_cap(EVENT_CAP - 1)]
#[case::at_cap(EVENT_CAP)]
fn event_at_cap(#[case] known_bytes: usize) {
    let limits = ApiLimits::default();
    let checked = limits
        .check_event(EventBoundary { known_bytes })
        .expect("provider events at or below the cap are accepted");
    assert_eq!(
        checked,
        ApiBoundariesResult {
            observed: known_bytes,
            limit: EVENT_CAP
        }
    );
}

#[rstest]
#[case::cap_plus_one(EVENT_CAP + 1)]
#[case::twice_the_cap(EVENT_CAP * 2)]
#[case::usize_max(usize::MAX)]
fn event_over_cap(#[case] known_bytes: usize) {
    let limits = ApiLimits::default();
    let error = limits
        .check_event(EventBoundary { known_bytes })
        .expect_err("a provider event over the cap is rejected");
    assert_eq!(
        error,
        ApiLimitError::EventTooLarge {
            observed: known_bytes,
            limit: EVENT_CAP,
        }
    );
    assert_eq!(
        error.fields(),
        LimitErrorFields {
            status: 502,
            error_type: "bad_gateway",
            param: "stream",
            code: "event_too_large",
        }
    );
}

#[rstest]
#[case::empty(0)]
#[case::one_item(1)]
#[case::one_below_cap(ITEMS_CAP - 1)]
#[case::at_cap(ITEMS_CAP)]
fn items_at_cap(#[case] known_count: usize) {
    let limits = ApiLimits::default();
    let checked = limits
        .check_items(ItemsBoundary { known_count })
        .expect("collections at or below the cap are accepted");
    assert_eq!(
        checked,
        ApiBoundariesResult {
            observed: known_count,
            limit: ITEMS_CAP
        }
    );
}

#[rstest]
#[case::cap_plus_one(ITEMS_CAP + 1)]
#[case::twice_the_cap(ITEMS_CAP * 2)]
#[case::usize_max(usize::MAX)]
fn items_over_cap(#[case] known_count: usize) {
    let limits = ApiLimits::default();
    let error = limits
        .check_items(ItemsBoundary { known_count })
        .expect_err("a collection over the cap is rejected");
    assert_eq!(
        error,
        ApiLimitError::TooManyItems {
            observed: known_count,
            limit: ITEMS_CAP,
        }
    );
    assert_eq!(
        error.fields(),
        LimitErrorFields {
            status: 400,
            error_type: "invalid_request_error",
            param: "input",
            code: "too_many_items",
        }
    );
}

#[rstest]
#[case::json(
    ApiBoundariesInput::Json(JsonBoundary { known_bytes: JSON_CAP + 1 }),
    ApiLimitError::JsonTooLarge { observed: JSON_CAP + 1, limit: JSON_CAP },
    LimitErrorFields { status: 413, error_type: "invalid_request_error", param: "body", code: "request_too_large" }
)]
#[case::event(
    ApiBoundariesInput::Event(EventBoundary { known_bytes: EVENT_CAP + 1 }),
    ApiLimitError::EventTooLarge { observed: EVENT_CAP + 1, limit: EVENT_CAP },
    LimitErrorFields { status: 502, error_type: "bad_gateway", param: "stream", code: "event_too_large" }
)]
#[case::items(
    ApiBoundariesInput::Items(ItemsBoundary { known_count: ITEMS_CAP + 1 }),
    ApiLimitError::TooManyItems { observed: ITEMS_CAP + 1, limit: ITEMS_CAP },
    LimitErrorFields { status: 400, error_type: "invalid_request_error", param: "input", code: "too_many_items" }
)]
fn compatibility_deviations(
    #[case] input: ApiBoundariesInput,
    #[case] expected: ApiLimitError,
    #[case] fields: LimitErrorFields,
) {
    let limits = ApiLimits::default();
    let error = limits
        .check(input)
        .expect_err("betterlite rejects input past its pinned cap");
    assert_eq!(error, expected);
    assert_eq!(error.fields(), fields);
    // the pinned responses iterator parses each event uncapped, so the same size passes without betterlite's caps
    let unconstrained = ApiLimits {
        max_json_bytes: usize::MAX,
        max_event_bytes: usize::MAX,
        max_items: usize::MAX,
    };
    assert!(unconstrained.check(input).is_ok());
}
