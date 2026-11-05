#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::sections::Section as _;
use crate::sections::aiohttp_transport_toggle::Raw;

#[rstest]
#[case::absent("litellm_settings: {}", &[], Some(false))]
#[case::yaml_flag("litellm_settings: {disable_aiohttp_transport: true}", &[], Some(true))]
#[case::environment_flag("litellm_settings: {}", &[("DISABLE_AIOHTTP_TRANSPORT", "True")], Some(true))]
#[case::empty_environment_is_unset("litellm_settings: {}", &[("DISABLE_AIOHTTP_TRANSPORT", "")], Some(false))]
#[case::bad_environment_names_the_variable("litellm_settings: {}", &[("DISABLE_AIOHTTP_TRANSPORT", "maybe")], None)]
fn accepts_the_aiohttp_transport_toggle_as_a_no_op(
    #[case] yaml: &str,
    #[case] variables: &[(&str, &str)],
    #[case] expected: Option<bool>,
) {
    let environment: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let raw: Raw = serde_saphyr::from_str(yaml).unwrap();
    let converted = raw.convert(&environment);
    assert_eq!(
        converted
            .ok()
            .map(|settings| settings.disable_aiohttp_transport),
        expected
    );
}
