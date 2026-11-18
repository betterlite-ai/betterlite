use crate::prometheus::failure::exception::LabelException;
use crate::prometheus::failure::status_code::extract_status_code;
use crate::prometheus::failure::tests::support::with_status;

#[test]
fn the_label_status_code_wins_over_both_exceptions() {
    let exception = with_status(LabelException::default(), Some("429"), None);
    let from_kwargs = with_status(LabelException::default(), Some("503"), None);
    let status_code = extract_status_code(Some("500"), Some(&exception), Some(&from_kwargs));
    assert_eq!(status_code, Some(500));
}

#[test]
fn an_unparsable_label_status_code_falls_through() {
    let exception = with_status(LabelException::default(), Some("429"), None);
    let status_code = extract_status_code(Some("server error"), Some(&exception), None);
    assert_eq!(status_code, Some(429));
}

#[test]
fn an_empty_label_status_code_falls_through() {
    let exception = with_status(LabelException::default(), Some("429"), None);
    let status_code = extract_status_code(Some(""), Some(&exception), None);
    assert_eq!(status_code, Some(429));
}

#[test]
fn the_exception_status_code_comes_before_the_kwargs_one() {
    let exception = with_status(LabelException::default(), Some("429"), None);
    let from_kwargs = with_status(LabelException::default(), Some("503"), None);
    let status_code = extract_status_code(None, Some(&exception), Some(&from_kwargs));
    assert_eq!(status_code, Some(429));
}

#[test]
fn the_exception_code_answers_when_its_status_code_is_absent() {
    let exception = with_status(LabelException::default(), None, Some("400"));
    let status_code = extract_status_code(None, Some(&exception), None);
    assert_eq!(status_code, Some(400));
}

#[test]
fn the_exception_code_answers_when_its_status_code_is_empty() {
    let exception = with_status(LabelException::default(), Some(""), Some("400"));
    let status_code = extract_status_code(None, Some(&exception), None);
    assert_eq!(status_code, Some(400));
}

#[test]
fn an_unparsable_exception_status_code_does_not_fall_back_to_its_code() {
    let exception = with_status(LabelException::default(), Some("not-a-code"), Some("400"));
    let status_code = extract_status_code(None, Some(&exception), None);
    assert_eq!(status_code, None);
}

#[test]
fn an_unparsable_exception_status_code_falls_to_the_kwargs_exception() {
    let exception = with_status(LabelException::default(), Some("not-a-code"), Some("400"));
    let from_kwargs = with_status(LabelException::default(), Some("503"), None);
    let status_code = extract_status_code(None, Some(&exception), Some(&from_kwargs));
    assert_eq!(status_code, Some(503));
}

#[test]
fn the_kwargs_exception_status_code_answers_last() {
    let from_kwargs = with_status(LabelException::default(), Some("503"), None);
    let status_code = extract_status_code(None, None, Some(&from_kwargs));
    assert_eq!(status_code, Some(503));
}

#[test]
fn the_kwargs_exception_code_answers_when_its_status_code_is_absent() {
    let from_kwargs = with_status(LabelException::default(), None, Some("400"));
    let status_code = extract_status_code(None, None, Some(&from_kwargs));
    assert_eq!(status_code, Some(400));
}

#[test]
fn an_empty_exception_code_is_no_status() {
    let exception = with_status(LabelException::default(), None, Some(""));
    let status_code = extract_status_code(None, Some(&exception), None);
    assert_eq!(status_code, None);
}

#[test]
fn nothing_resolvable_is_no_status() {
    assert_eq!(extract_status_code(None, None, None), None);
}
