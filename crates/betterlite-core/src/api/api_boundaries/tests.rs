#![cfg(test)]

use rstest::rstest;

use super::{
    ApiBoundariesInput, ApiLimitError, ApiLimits, EventBoundary, ItemsBoundary, JsonBoundary,
    LimitErrorFields,
};

// the caps are literals here (0x0001_0000 is 65536), so a mutated bound constant cannot move the test with it
const JSON_CAP: usize = 16 * 1024 * 1024;
const EVENT_CAP: usize = 1024 * 1024;
const ITEMS_CAP: usize = 0x0001_0000;

const JSON_FIELDS: LimitErrorFields = LimitErrorFields {
    status: 413,
    error_type: "invalid_request_error",
    param: "body",
    code: "request_too_large",
};

const EVENT_FIELDS: LimitErrorFields = LimitErrorFields {
    status: 502,
    error_type: "bad_gateway",
    param: "stream",
    code: "event_too_large",
};

const ITEMS_FIELDS: LimitErrorFields = LimitErrorFields {
    status: 400,
    error_type: "invalid_request_error",
    param: "input",
    code: "too_many_items",
};

fn json(known_bytes: usize) -> ApiBoundariesInput {
    ApiBoundariesInput::Json(JsonBoundary { known_bytes })
}

fn event(known_bytes: usize) -> ApiBoundariesInput {
    ApiBoundariesInput::Event(EventBoundary { known_bytes })
}

fn items(known_count: usize) -> ApiBoundariesInput {
    ApiBoundariesInput::Items(ItemsBoundary { known_count })
}

#[rstest]
#[case::empty(0)]
#[case::one_byte(1)]
#[case::one_below_cap(JSON_CAP - 1)]
#[case::at_cap(JSON_CAP)]
fn json_at_cap(#[case] known_bytes: usize) {
    let checked = ApiLimits::default()
        .check(json(known_bytes))
        .expect("accepted");
    assert_eq!(checked.observed, known_bytes);
    assert_eq!(checked.limit, JSON_CAP);
}

#[rstest]
#[case::cap_plus_one(JSON_CAP + 1)]
#[case::twice_the_cap(JSON_CAP * 2)]
#[case::usize_max(usize::MAX)]
fn json_over_cap(#[case] known_bytes: usize) {
    let error = ApiLimits::default()
        .check(json(known_bytes))
        .expect_err("rejected");
    assert_eq!(
        error,
        ApiLimitError::JsonTooLarge {
            observed: known_bytes,
            limit: JSON_CAP
        }
    );
    assert_eq!(error.fields(), JSON_FIELDS);
}

#[rstest]
#[case::empty(0)]
#[case::one_byte(1)]
#[case::one_below_cap(EVENT_CAP - 1)]
#[case::at_cap(EVENT_CAP)]
fn event_at_cap(#[case] known_bytes: usize) {
    let checked = ApiLimits::default()
        .check(event(known_bytes))
        .expect("accepted");
    assert_eq!(checked.observed, known_bytes);
    assert_eq!(checked.limit, EVENT_CAP);
}

#[rstest]
#[case::cap_plus_one(EVENT_CAP + 1)]
#[case::twice_the_cap(EVENT_CAP * 2)]
#[case::usize_max(usize::MAX)]
fn event_over_cap(#[case] known_bytes: usize) {
    let error = ApiLimits::default()
        .check(event(known_bytes))
        .expect_err("rejected");
    assert_eq!(
        error,
        ApiLimitError::EventTooLarge {
            observed: known_bytes,
            limit: EVENT_CAP
        }
    );
    assert_eq!(error.fields(), EVENT_FIELDS);
}

#[rstest]
#[case::empty(0)]
#[case::one_item(1)]
#[case::one_below_cap(ITEMS_CAP - 1)]
#[case::at_cap(ITEMS_CAP)]
fn items_at_cap(#[case] known_count: usize) {
    let checked = ApiLimits::default()
        .check(items(known_count))
        .expect("accepted");
    assert_eq!(checked.observed, known_count);
    assert_eq!(checked.limit, ITEMS_CAP);
}

#[rstest]
#[case::cap_plus_one(ITEMS_CAP + 1)]
#[case::twice_the_cap(ITEMS_CAP * 2)]
#[case::usize_max(usize::MAX)]
fn items_over_cap(#[case] known_count: usize) {
    let error = ApiLimits::default()
        .check(items(known_count))
        .expect_err("rejected");
    assert_eq!(
        error,
        ApiLimitError::TooManyItems {
            observed: known_count,
            limit: ITEMS_CAP
        }
    );
    assert_eq!(error.fields(), ITEMS_FIELDS);
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
        .expect_err("rejected past the pinned cap");
    assert_eq!(error, expected);
    assert_eq!(error.fields(), fields);
    // the pinned responses iterator parses each event uncapped, so the same size passes without betterlite's caps
    let unconstrained = ApiLimits {
        max_json_bytes: usize::MAX,
        max_event_bytes: usize::MAX,
        max_items: usize::MAX,
    };
    let unchecked = unconstrained
        .check(input)
        .expect("the pinned behavior has no cap");
    assert_eq!(unchecked.limit, usize::MAX);
}
