use crate::file_ids::managed_id::get_format_from_file_id;
use rstest::rstest;

const ENCODED_FORMAT_ID: &str = "bGl0ZWxsbV9wcm94eTpmaWxlL3BkZjt1bmlmaWVkX2lkLGFiYw";
const STANDARD_ALPHABET_ID: &str = "bGl0ZWxsbV9wcm94eTpmaWxl8J+YgDt1bmlmaWVkX2lkLHU";
const URL_SAFE_ALPHABET_ID: &str = "bGl0ZWxsbV9wcm94eTpmaWxlLmFiY_CfmIA-O3VuaWZpZWRfaWQsYWJj";
const DOTTED_FORMAT_ID: &str = "bGl0ZWxs....bV9wcm94eTpmaWxlL3BkZjt1bmlmaWVkX2lkLGFiYw";

#[rstest]
#[case::absent_id(None, None)]
#[case::empty_id(Some(""), None)]
#[case::plain_provider_id(Some("file-abc123"), None)]
#[case::raw_unified_id(Some("litellm_proxy:file/pdf;unified_id,abc"), Some("file/pdf"))]
#[case::encoded_unified_id(Some(ENCODED_FORMAT_ID), Some("file/pdf"))]
#[case::standard_base64_id(Some(STANDARD_ALPHABET_ID), Some("file\u{1F600}"))]
#[case::url_safe_alphabet_id(Some(URL_SAFE_ALPHABET_ID), Some("file.abc\u{1F600}>"))]
#[case::characters_outside_the_alphabet_are_discarded(Some(DOTTED_FORMAT_ID), Some("file/pdf"))]
#[case::empty_embedded_format(Some("litellm_proxy:;unified_id,abc"), Some(""))]
#[case::unified_id_without_the_unified_field(Some("litellm_proxy:file/pdf;other,abc"), None)]
#[case::format_carrying_a_newline_is_no_match(Some("litellm_proxy:file\npdf;unified_id,abc"), None)]
fn model_file_ids_behavior_tests_behavior(
    #[case] file_id: Option<&str>,
    #[case] expected: Option<&str>,
) {
    assert_eq!(get_format_from_file_id(file_id).as_deref(), expected);
}
