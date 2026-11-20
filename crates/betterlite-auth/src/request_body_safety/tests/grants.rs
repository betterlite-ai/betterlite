use betterlite_config::sections::request_body_safety::ClientsideParamGrant;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use rstest::rstest;

use crate::request_body_safety::SafetyError;
use crate::request_body_safety::grants::{check_regex_or_str_match, is_param_allowed};

use super::{Fixture, denied};

fn matcher(pattern: Json) -> Vec<ClientsideParamGrant> {
    let mut fields = JsonObject::new();
    fields.insert("api_base".to_owned(), pattern);
    vec![ClientsideParamGrant::Matcher(fields)]
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

#[test]
fn regex_prefix_before_equality() {
    assert!(check_regex_or_str_match("https://ok.example/v1", "^https://ok.example").unwrap());
    // re.match anchors at the start, not the end
    assert!(check_regex_or_str_match("abcdef", "abc").unwrap());
    assert!(!check_regex_or_str_match("xabc", "abc").unwrap());
    // exact equality still admits a value the pattern does not match
    assert!(!check_regex_or_str_match("a+b", "^https://").unwrap());
    assert!(check_regex_or_str_match("^literal", "^literal").unwrap());
    // the regex runs first, so a malformed pattern raises even when it equals the value
    assert!(matches!(
        check_regex_or_str_match("(", "("),
        Err(SafetyError::MatcherRegex(_))
    ));

    let items = matcher(text("^https://ok.example"));
    assert!(is_param_allowed("api_base", &text("https://ok.example/v1"), &items).unwrap());
    assert!(!is_param_allowed("api_base", &text("https://no.example"), &items).unwrap());
    // a mapping entry is considered only for api_base
    assert!(!is_param_allowed("aws_role_name", &text("https://ok.example"), &items).unwrap());
    // a string entry grants only its own parameter
    let exact = vec![ClientsideParamGrant::Exact("aws_role_name".to_owned())];
    assert!(is_param_allowed("aws_role_name", &text("role"), &exact).unwrap());
    assert!(!is_param_allowed("aws_session_name", &text("role"), &exact).unwrap());
}

#[test]
fn regex_type_key_failures() {
    let mut missing = JsonObject::new();
    missing.insert("other".to_owned(), text("x"));
    let without_key = vec![ClientsideParamGrant::Matcher(missing)];
    assert!(matches!(
        is_param_allowed("api_base", &text("https://ok"), &without_key),
        Err(SafetyError::MatcherMissing)
    ));
    assert!(matches!(
        is_param_allowed(
            "api_base",
            &text("https://ok"),
            &matcher(Json::Number(serde_json::Number::from(7_i32)))
        ),
        Err(SafetyError::MatcherType)
    ));
    assert!(matches!(
        is_param_allowed("api_base", &Json::Null, &matcher(text("^https://ok"))),
        Err(SafetyError::MatcherType)
    ));
    assert!(matches!(
        is_param_allowed("api_base", &text("https://ok"), &matcher(text("("))),
        Err(SafetyError::MatcherRegex(_))
    ));
    // a missing key answers before the request value's own type is inspected
    assert!(matches!(
        is_param_allowed("api_base", &Json::Null, &without_key),
        Err(SafetyError::MatcherMissing)
    ));
}

#[rstest]
#[case::banned(denied(&Fixture::default(), r#"{"api_base": "https://evil.example"}"#))]
#[case::model_list(SafetyError::ModelList)]
#[case::fallback_depth(SafetyError::FallbackDepth)]
fn value_error_401_envelope(#[case] error: SafetyError) {
    let exception = error.into_proxy_exception();
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
    assert!(
        exception
            .message
            .starts_with("Authentication Error, Rejected Request:"),
        "{}",
        exception.message
    );
    assert_eq!(exception.code.parse::<u16>(), Ok(401));
}
