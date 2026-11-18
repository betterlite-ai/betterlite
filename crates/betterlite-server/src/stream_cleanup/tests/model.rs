use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::super::model::{client_named_model, later_chunk_model, restore_partial_model};
use super::support::{number, object, text};

#[rstest]
#[case::different_models(vec![object(&[("model", text("alias"))])], Some(text("routed")), true)]
#[case::same_model(vec![object(&[("model", text("alias"))])], Some(text("alias")), false)]
#[case::empty_assembled(vec![object(&[("model", text("alias"))])], Some(text("")), false)]
#[case::no_assembled_model(vec![object(&[("model", text("alias"))])], None, false)]
#[case::numeric_assembled(vec![object(&[("model", text("alias"))])], Some(number(7)), false)]
#[case::chunk_without_model(vec![object(&[])], Some(text("routed")), false)]
#[case::numeric_chunk_model(vec![object(&[("model", number(7))])], Some(text("routed")), false)]
#[case::no_chunks(Vec::new(), Some(text("routed")), false)]
fn later_chunk_model_matches_litellm(
    #[case] chunks: Vec<JsonObject>,
    #[case] assembled: Option<Json>,
    #[case] expected: bool,
) {
    assert_eq!(later_chunk_model(&chunks, assembled.as_ref()), expected);
}

#[rstest]
#[case::requested_name_matches(object(&[("_litellm_client_requested_model", text("alias"))]), Some(text("alias")), true)]
#[case::requested_name_beats_model(object(&[("_litellm_client_requested_model", text("alias")), ("model", text("other"))]), Some(text("other")), false)]
#[case::falls_back_to_model(object(&[("_litellm_client_requested_model", number(5)), ("model", text("alias"))]), Some(text("alias")), true)]
#[case::missing_model(object(&[]), Some(text("alias")), false)]
#[case::numeric_model(object(&[("model", number(5))]), Some(text("alias")), false)]
#[case::no_assembled_model(object(&[("model", text("alias"))]), None, false)]
#[case::different_name(object(&[("model", text("alias"))]), Some(text("other")), false)]
fn client_named_model_matches_litellm(
    #[case] request_data: JsonObject,
    #[case] assembled: Option<Json>,
    #[case] expected: bool,
) {
    assert_eq!(
        client_named_model(&request_data, assembled.as_ref()),
        expected
    );
}

#[rstest]
#[case::wrapper_replaces_matching_model(
    vec![object(&[("model", text("alias"))])],
    object(&[("model", text("alias"))]),
    Some("gpt-4o"),
    object(&[]),
    "gpt-4o"
)]
#[case::routed_model_kept(
    vec![object(&[("model", text("alias"))])],
    object(&[("model", text("routed"))]),
    Some("gpt-4o"),
    object(&[]),
    "routed"
)]
#[case::client_named_routed_model_gets_the_wrapper(
    vec![object(&[("model", text("alias"))])],
    object(&[("model", text("routed"))]),
    Some("gpt-4o"),
    object(&[("_litellm_client_requested_model", text("routed"))]),
    "gpt-4o"
)]
#[case::empty_wrapper_keeps_the_model(
    vec![object(&[("model", text("alias"))])],
    object(&[("model", text("alias"))]),
    Some(""),
    object(&[]),
    "alias"
)]
#[case::missing_wrapper_keeps_the_model(
    vec![object(&[("model", text("alias"))])],
    object(&[("model", text("alias"))]),
    None,
    object(&[]),
    "alias"
)]
#[case::missing_model_gets_the_wrapper(
    vec![object(&[("model", text("alias"))])],
    object(&[]),
    Some("gpt-4o"),
    object(&[]),
    "gpt-4o"
)]
#[case::numeric_model_gets_the_wrapper(
    vec![object(&[("model", text("alias"))])],
    object(&[("model", number(7))]),
    Some("gpt-4o"),
    object(&[]),
    "gpt-4o"
)]
fn restore_partial_model_puts_the_wrapper_name_back(
    #[case] chunks: Vec<JsonObject>,
    #[case] partial: JsonObject,
    #[case] wrapper: Option<&str>,
    #[case] request_data: JsonObject,
    #[case] expected: &str,
) {
    let mut working = partial;
    restore_partial_model(&mut working, &chunks, wrapper, &request_data);
    assert_eq!(working.get("model"), Some(&text(expected)));
}
