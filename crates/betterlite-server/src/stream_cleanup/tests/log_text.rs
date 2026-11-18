use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use super::super::log_text::call_id_text;
use super::support::{number, object, text};

#[rstest]
#[case::text_id(object(&[("litellm_call_id", text("call-1"))]), "call-1")]
#[case::numeric_id(object(&[("litellm_call_id", number(7))]), "None")]
#[case::missing_id(object(&[]), "None")]
fn call_id_text_shows_the_requested_id_or_none(
    #[case] request_data: JsonObject,
    #[case] expected: &str,
) {
    assert_eq!(call_id_text(&request_data), expected);
}
