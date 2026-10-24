#![cfg(test)]

use std::io::Cursor;

use crate::case::HelperCase;

#[test]
fn b171_helper_runner_tests_behavior() {
    let case: HelperCase = serde_saphyr::from_str(
        "kind: helper\nname: reserved\ncall: {url_path: {function: segment, value: .}}\nenv: {LITELLM_LOCAL_MODEL_COST_MAP: 'False'}\ncompare: {ignore: []}\n",
    )
    .expect("helper case should parse");
    assert!(
        matches!(&case.call, super::HelperCall::UrlPath(input) if input.field_name == "path parameter")
    );
    assert!(matches!(
        super::run_case(&case, false).expect("helper should run"),
        super::HelperRun::Match
    ));
}

#[rstest::rstest]
#[case::at_limit(0x0010_0000, false)]
#[case::over_limit(0x0010_0001, true)]
fn helper_output_limit_is_enforced(#[case] size: usize, #[case] excess: bool) {
    let bytes = vec![b'x'; size];
    let (output, observed_excess) = super::read_limited(Cursor::new(bytes)).unwrap();
    assert_eq!(output.len(), 0x0010_0000.min(size));
    assert_eq!(observed_excess, excess);
}
