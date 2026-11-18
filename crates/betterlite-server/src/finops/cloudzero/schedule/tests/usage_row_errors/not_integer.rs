use super::*;

#[test]
fn cloudzero_schedule() {
    let not_integer =
        usage_record(&row(&[("prompt_tokens", text("ten"))])).expect_err("text is not an integer");
    assert_eq!(
        not_integer.to_string(),
        "the usage row field 'prompt_tokens' is not an integer"
    );
    assert!(matches!(
        not_integer,
        UsageRowError::NotInteger {
            field: "prompt_tokens"
        }
    ));

    let fractional = usage_record(&row(&[("api_requests", decimal("1.5"))]))
        .expect_err("a fraction is not an integer");
    assert!(matches!(
        fractional,
        UsageRowError::NotInteger {
            field: "api_requests"
        }
    ));
}
