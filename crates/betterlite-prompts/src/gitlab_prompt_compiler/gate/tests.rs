#![cfg(test)]

use betterlite_models::decisions::Json as ContractJson;
use rstest::rstest;

use super::should_run_prompt_management;

#[rstest]
#[case::prompt_id(Some("chat/hi"), true)]
#[case::empty_prompt_id(Some(""), true)]
#[case::no_prompt_id(None, false)]
fn a_prompt_id_decides_whether_prompt_management_runs(
    #[case] prompt_id: Option<&str>,
    #[case] expected: bool,
) {
    let dynamic_callback_params = ContractJson::Null;

    assert_eq!(
        should_run_prompt_management(prompt_id, None, &dynamic_callback_params),
        expected
    );
}
