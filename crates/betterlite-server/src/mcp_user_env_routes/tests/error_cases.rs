use std::convert::Infallible;

use crate::mcp_user_env_routes::body::BodyRefusal;
use crate::mcp_user_env_routes::error::{HttpRefusal, UserEnvRouteError};

type TestError = UserEnvRouteError<Infallible, Infallible>;

#[test]
fn a_validation_refusal_reports_a_fastapi_validation_error() {
    let error = TestError::Body(BodyRefusal::Validation(Vec::new()));
    let facts = error.facts();
    assert_eq!(facts.class_name, "RequestValidationError");
    assert_eq!(facts.status_code, Some(422));
    assert_eq!(facts.response_status_code, Some(422));
}

#[test]
fn an_unreadable_body_reports_an_http_exception() {
    let error = TestError::Body(BodyRefusal::Unreadable);
    let facts = error.facts();
    assert_eq!(facts.class_name, "HTTPException");
    assert_eq!(facts.status_code, Some(400));
}

#[test]
fn a_refusal_reports_its_detail_message() {
    let error = TestError::Refusal(HttpRefusal::missing_user_id());
    assert_eq!(error.to_string(), "User ID not found in token");
    let facts = error.facts();
    assert_eq!(facts.class_name, "HTTPException");
    assert_eq!(facts.status_code, Some(400));
}
