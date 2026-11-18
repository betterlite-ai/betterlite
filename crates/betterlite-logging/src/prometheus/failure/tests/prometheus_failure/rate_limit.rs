use crate::prometheus::failure::exception::LabelException;
use crate::prometheus::failure::rate_limit::extract_rate_limit_labels;
use crate::prometheus::failure::tests::support::with_rate_limit;

#[test]
fn no_exception_is_no_labels() {
    assert_eq!(extract_rate_limit_labels(None), (None, None));
}

#[test]
fn a_known_category_and_type_pass_validation() {
    let exception = with_rate_limit(LabelException::default(), "vendor_rate_limit", "requests");
    assert_eq!(
        extract_rate_limit_labels(Some(&exception)),
        (
            Some(String::from("vendor_rate_limit")),
            Some(String::from("requests"))
        )
    );
}

#[test]
fn an_unknown_category_and_type_are_dropped_together() {
    let exception = with_rate_limit(LabelException::default(), "nonsense", "widgets");
    assert_eq!(extract_rate_limit_labels(Some(&exception)), (None, None));
}

#[test]
fn an_unknown_type_is_dropped_on_its_own() {
    let exception = with_rate_limit(LabelException::default(), "litellm_rate_limit", "widgets");
    assert_eq!(
        extract_rate_limit_labels(Some(&exception)),
        (Some(String::from("litellm_rate_limit")), None)
    );
}

#[test]
fn an_unknown_category_is_dropped_on_its_own() {
    let exception = with_rate_limit(LabelException::default(), "nonsense", "budget");
    assert_eq!(
        extract_rate_limit_labels(Some(&exception)),
        (None, Some(String::from("budget")))
    );
}

#[test]
fn absent_fields_are_no_labels() {
    let exception = LabelException::default();
    assert_eq!(extract_rate_limit_labels(Some(&exception)), (None, None));
}

#[test]
fn empty_fields_are_no_labels() {
    let exception = with_rate_limit(LabelException::default(), "", "");
    assert_eq!(extract_rate_limit_labels(Some(&exception)), (None, None));
}
