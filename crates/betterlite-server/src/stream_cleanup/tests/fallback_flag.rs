use betterlite_config::sections::fallback_errors::FallbackErrors;
use betterlite_models::decisions::Json;

use super::super::fallback_flag::drop_fallback_errors_flag;
use super::support::{object, text};

#[test]
fn drop_fallback_errors_flag_removes_the_flag_by_default() {
    let mut data = object(&[
        ("include_fallback_errors", Json::Bool(true)),
        ("model", text("gpt-4o")),
    ]);
    drop_fallback_errors_flag(&FallbackErrors::default(), &mut data);
    assert_eq!(data, object(&[("model", text("gpt-4o"))]));
}

#[test]
fn drop_fallback_errors_flag_keeps_the_flag_when_exposed() {
    let mut data = object(&[
        ("include_fallback_errors", Json::Bool(true)),
        ("model", text("gpt-4o")),
    ]);
    drop_fallback_errors_flag(
        &FallbackErrors {
            expose_to_caller: true,
        },
        &mut data,
    );
    assert_eq!(
        data,
        object(&[
            ("include_fallback_errors", Json::Bool(true)),
            ("model", text("gpt-4o")),
        ])
    );
}

#[test]
fn drop_fallback_errors_flag_ignores_data_without_the_flag() {
    let mut data = object(&[("model", text("gpt-4o"))]);
    drop_fallback_errors_flag(&FallbackErrors::default(), &mut data);
    assert_eq!(data, object(&[("model", text("gpt-4o"))]));
}
