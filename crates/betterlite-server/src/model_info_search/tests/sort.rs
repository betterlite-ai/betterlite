use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::support::{decimal_text, flag, model, names, number, object, text};
use crate::model_info_search::sort::sort_models;

fn created(name: &str, value: Json) -> JsonObject {
    model(name, vec![("created_at", value)])
}

fn costs(name: &str, input: Json, output: Json) -> JsonObject {
    model(
        name,
        vec![
            ("input_cost_per_token", input),
            ("output_cost_per_token", output),
        ],
    )
}

#[test]
fn date_sort() {
    let models = vec![
        created("b", text("2026-01-02T00:00:00Z")),
        created("a", text("2026-01-01T00:00:00Z")),
        created("c", text("2025-12-31T18:30:00+05:30")),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), None)),
        ["c", "a", "b"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), Some("desc"))),
        ["b", "a", "c"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), Some("DESC"))),
        ["b", "a", "c"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), Some("asc"))),
        ["c", "a", "b"]
    );
    let updated = vec![
        model("b", vec![("updated_at", text("2026-01-02T00:00:00Z"))]),
        model("a", vec![("updated_at", text("2026-01-01T00:00:00Z"))]),
    ];
    assert_eq!(
        names(&sort_models(&updated, Some("updated_at"), None)),
        ["a", "b"]
    );
}

#[test]
fn invalid_date_fallback() {
    let models = vec![
        created("bad-feb", text("2026-02-30T00:00:00")),
        created("good", text("2026-01-01T00:00:00")),
        created("bad-text", text("not-a-date")),
        created("missing", Json::Null),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), None)),
        ["good", "bad-feb", "bad-text", "missing"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), Some("desc"))),
        ["good", "bad-feb", "bad-text", "missing"]
    );
}

#[test]
fn invalid_dates_only_keeps_input_order() {
    let models = vec![
        created("first", text("nope")),
        created("second", number(7)),
        created("third", text("2026-02-30T00:00:00")),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), None)),
        ["first", "second", "third"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), Some("desc"))),
        ["first", "second", "third"]
    );
}

#[test]
fn cost_sort_uses_decimal_totals() {
    let models = vec![
        costs("high", decimal_text("0.5"), decimal_text("0.5")),
        costs("low", decimal_text("0.000001"), Json::Null),
        costs("negative", number(-1), number(0)),
        model("absent", Vec::new()),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("costs"), None)),
        ["negative", "low", "high", "absent"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("costs"), Some("desc"))),
        ["high", "low", "negative", "absent"]
    );
}

#[test]
fn zero_cost_sorts_last_both_directions() {
    let models = vec![
        costs("zero", number(0), number(0)),
        costs("priced", number(1), number(0)),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("costs"), None)),
        ["priced", "zero"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("costs"), Some("desc"))),
        ["priced", "zero"]
    );
}

#[test]
fn cost_sort_ties_are_decimal_exact() {
    let models = vec![
        costs("tenths", decimal_text("0.1"), decimal_text("0.2")),
        costs("direct", decimal_text("0.3"), number(0)),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("costs"), None)),
        ["tenths", "direct"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("costs"), Some("desc"))),
        ["tenths", "direct"]
    );
}

#[test]
fn status_sort_puts_config_deployments_first() {
    let models = vec![
        model("db", vec![("db_model", flag(true))]),
        model("config", Vec::new()),
        model("null", vec![("db_model", Json::Null)]),
        model("false", vec![("db_model", flag(false))]),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("status"), None)),
        ["config", "null", "false", "db"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("status"), Some("desc"))),
        ["db", "config", "null", "false"]
    );
}

#[test]
fn public_name_sort_prefers_the_displayed_name() {
    let models = vec![
        model("zeta", vec![("team_public_model_name", text("aaa"))]),
        model("beta", Vec::new()),
        model("gamma", vec![("team_public_model_name", text(""))]),
        model("delta", vec![("team_public_model_name", Json::Null)]),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("model_name"), None)),
        ["zeta", "beta", "delta", "gamma"]
    );
}

#[rstest]
#[case::unknown(Some("key"))]
#[case::empty(Some(""))]
#[case::absent(None)]
fn unsupported_fields_keep_input_order(#[case] sort_by: Option<&str>) {
    let models = vec![
        model("second", vec![("created_at", text("2026-01-02"))]),
        model("first", vec![("created_at", text("2026-01-01"))]),
    ];
    assert_eq!(
        names(&sort_models(&models, sort_by, None)),
        ["second", "first"]
    );
    assert_eq!(
        names(&sort_models(&models, sort_by, Some("desc"))),
        ["second", "first"]
    );
}

#[test]
fn a_sort_key_error_keeps_input_order() {
    let models = vec![
        object(vec![
            ("model_name", number(7)),
            ("litellm_params", Json::Object(object(Vec::new()))),
            ("model_info", Json::Object(object(Vec::new()))),
        ]),
        model("a", Vec::new()),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("model_name"), None)),
        ["", "a"]
    );
    let broken = vec![
        model("fine", Vec::new()),
        object(vec![
            ("model_name", text("broken")),
            ("litellm_params", Json::Object(object(Vec::new()))),
            ("model_info", text("not an object")),
        ]),
    ];
    assert_eq!(
        names(&sort_models(&broken, Some("costs"), None)),
        ["fine", "broken"]
    );
}

#[test]
fn equal_keys_keep_input_order_both_directions() {
    let models = vec![
        created("first", text("2026-01-01T00:00:00Z")),
        created("second", text("2026-01-01T00:00:00Z")),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), None)),
        ["first", "second"]
    );
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), Some("desc"))),
        ["first", "second"]
    );
}

#[test]
fn a_null_model_info_sorts_as_an_empty_one() {
    let models = vec![
        model("null", vec![("created_at", Json::Null)]),
        object(vec![
            ("model_name", text("missing")),
            ("litellm_params", Json::Object(object(Vec::new()))),
            ("model_info", Json::Null),
        ]),
        created("dated", text("2026-01-01")),
    ];
    assert_eq!(
        names(&sort_models(&models, Some("created_at"), None)),
        ["dated", "null", "missing"]
    );
}
