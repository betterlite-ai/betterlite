use rstest::rstest;

use crate::model_recognition::kwargs::{CallbackKwargs, model_group_from_kwargs};

fn callback_kwargs(source: &str) -> CallbackKwargs {
    serde_json::from_str(source).unwrap()
}

#[rstest]
#[case::metadata(
    r#"{"litellm_params":{"metadata":{"model_group":"gpt-4"}}}"#,
    Some("gpt-4")
)]
#[case::litellm_metadata(
    r#"{"litellm_metadata":{},"litellm_params":{"litellm_metadata":{"model_group":"gpt-4"}}}"#,
    Some("gpt-4")
)]
#[case::newer_bucket_wins_even_when_null(
    r#"{"litellm_metadata":null,"litellm_params":{"metadata":{"model_group":"old"},"litellm_metadata":{"model_group":"new"}}}"#,
    Some("new")
)]
#[case::empty_group(r#"{"litellm_params":{"metadata":{"model_group":""}}}"#, Some(""))]
#[case::no_litellm_params("{}", None)]
#[case::null_litellm_params(r#"{"litellm_params":null}"#, None)]
#[case::null_metadata(r#"{"litellm_params":{"metadata":null}}"#, None)]
#[case::absent_metadata(r#"{"litellm_params":{}}"#, None)]
#[case::empty_metadata(r#"{"litellm_params":{"metadata":{}}}"#, None)]
#[case::absent_model_group(r#"{"litellm_params":{"metadata":{"other":"value"}}}"#, None)]
#[case::null_model_group(r#"{"litellm_params":{"metadata":{"model_group":null}}}"#, None)]
#[case::metadata_ignored_when_the_call_carries_litellm_metadata(
    r#"{"litellm_metadata":{},"litellm_params":{"metadata":{"model_group":"old"}}}"#,
    None
)]
#[case::litellm_metadata_ignored_without_the_top_level_key(
    r#"{"litellm_params":{"litellm_metadata":{"model_group":"new"}}}"#,
    None
)]
fn model_group_reads_the_selected_metadata_bucket(
    #[case] source: &str,
    #[case] expected: Option<&str>,
) {
    let kwargs = callback_kwargs(source);
    assert_eq!(model_group_from_kwargs(&kwargs), expected);
}

#[rstest]
#[case::params_list(r#"{"litellm_params":[]}"#)]
#[case::params_text(r#"{"litellm_params":"params"}"#)]
#[case::metadata_number(r#"{"litellm_params":{"metadata":7}}"#)]
#[case::litellm_metadata_text(
    r#"{"litellm_metadata":{},"litellm_params":{"litellm_metadata":"meta"}}"#
)]
#[case::model_group_number(r#"{"litellm_params":{"metadata":{"model_group":7}}}"#)]
#[case::model_group_list(r#"{"litellm_params":{"metadata":{"model_group":["a"]}}}"#)]
fn callback_kwargs_reject_a_value_the_field_cannot_hold(#[case] source: &str) {
    assert!(serde_json::from_str::<CallbackKwargs>(source).is_err());
}
