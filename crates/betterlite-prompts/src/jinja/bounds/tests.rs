#![cfg(test)]

use super::{OUTPUT_BYTES, TEMPLATE_BYTES, check_output, check_template};
use rstest::rstest;

#[derive(Clone, Copy)]
enum Boundary {
    Template,
    Output,
}

#[rstest]
#[case::template_exact(Boundary::Template, TEMPLATE_BYTES, true)]
#[case::template_over(Boundary::Template, TEMPLATE_BYTES + 1, false)]
#[case::output_exact(Boundary::Output, OUTPUT_BYTES, true)]
#[case::output_over(Boundary::Output, OUTPUT_BYTES + 1, false)]
fn jinja_prompts_behavior_tests_behavior(
    #[case] boundary: Boundary,
    #[case] size: usize,
    #[case] accepted: bool,
) {
    let text = "x".repeat(size);
    match boundary {
        Boundary::Template => assert_eq!(check_template(&text).is_ok(), accepted),
        Boundary::Output => assert_eq!(check_output(text).is_ok(), accepted),
    }
}
