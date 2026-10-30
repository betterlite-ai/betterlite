use super::*;

#[rstest]
#[case::plain("APIConnectionError")]
#[case::missing("APIError")]
fn error_facts_behavior_tests_behavior(#[case] name: &str) {
    let text = format!("litellm.{name}");
    let result = extract_exception(None, &text, "model", "provider", None);
    if name == "APIConnectionError" {
        assert_eq!(
            result.unwrap().map(|facts| facts.name),
            Some(name.to_owned())
        );
    } else {
        assert!(matches!(
            result,
            Err(ParseError::Constructor { name: error_name }) if error_name == name
        ));
    }
}
