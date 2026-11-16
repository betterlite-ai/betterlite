use betterlite_openai::response_input::ResponsesInput;
use rstest::rstest;

use super::support;
use crate::responses::error::ResponsesError;
use crate::responses::request::transform_request;

#[test]
fn a_non_object_function_maps_to_a_bad_request() {
    let error = ResponsesError::InvalidToolFunction {
        index: 2,
        found: "NoneType",
    };
    assert_eq!(error.http_status(), Some(400));
    assert_eq!(
        error.to_string(),
        "invalid type for 'tools[2].function': expected an object, but got NoneType instead"
    );
}

#[rstest]
#[case::null_function("null", "NoneType")]
#[case::string_function(r#""f""#, "str")]
#[case::array_function(r#"["f"]"#, "list")]
fn a_non_object_function_is_rejected_before_the_request(
    #[case] function: &str,
    #[case] found: &str,
) {
    let options = support::object(&format!(
        r#"{{"tools":[{{"type":"function","function":{function}}}]}}"#
    ));
    let error = transform_request(
        "gpt-5",
        &ResponsesInput::Text(String::from("hello")),
        &options,
        None,
    )
    .unwrap_err();
    let ResponsesError::InvalidToolFunction {
        index,
        found: actual,
    } = &error
    else {
        panic!("a non-object function must be refused, not {error}");
    };
    assert_eq!(*index, 0);
    assert_eq!(*actual, found);
    assert_eq!(error.http_status(), Some(400));
    assert!(error.to_string().contains("tools[0].function"));
}
