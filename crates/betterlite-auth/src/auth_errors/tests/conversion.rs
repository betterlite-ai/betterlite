use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ptr;

use betterlite_db::database_faults::ErrorType;
use betterlite_error::error::details::BudgetExceededDetails;
use betterlite_error::proxy_exception::{ExceptionCode, ProxyException, ProxyExceptionInit};
use betterlite_models::decisions::Json;
use rstest::rstest;
use rust_decimal::Decimal;

use super::support::{CLIENT_IP, database, identity_with, object, wire};
use crate::auth_errors::client_context::{
    escape_control_chars, identity_log_suffix, with_client_context,
};
use crate::auth_errors::conversion::{
    AuthFailure, ClientFailure, as_proxy_exception, budget_details_exception,
    other_message_exception,
};

#[test]
fn conversion_order_budget_default_422_and_absent_status_429() {
    let budget = |status_code| AuthFailure::BudgetExceeded {
        message: "Budget has been exceeded!".to_owned(),
        status_code,
        llm_provider: String::new(),
    };
    let configured = as_proxy_exception(&budget(Some(422_u16)));
    assert_eq!(
        wire(&configured),
        ("budget_exceeded".to_owned(), None, "422".to_owned())
    );
    assert_eq!(configured.exception().message, "Budget has been exceeded!");
    let absent = as_proxy_exception(&budget(None));
    assert_eq!(
        wire(&absent),
        ("budget_exceeded".to_owned(), None, "429".to_owned())
    );
}

#[rstest]
#[case::absent(None, "Budget has been exceeded! Current cost: 6, Max budget: 5")]
#[case::empty(Some(""), "Budget has been exceeded! Current cost: 6, Max budget: 5")]
#[case::given(Some("Budget has been exceeded!"), "Budget has been exceeded!")]
fn budget_details_exception_message_and_status(
    #[case] message: Option<&str>,
    #[case] expected: &str,
) {
    let details = BudgetExceededDetails {
        current_cost: Decimal::new(6, 0),
        max_budget: Decimal::new(5, 0),
        message: message.map(str::to_owned),
        provider: "openai".to_owned(),
        entity_type: Some("key".to_owned()),
        entity_id: Some("hashed-key".to_owned()),
        status_code: 422,
    };
    let exception = budget_details_exception(&details);
    assert_eq!(exception.message, expected);
    assert_eq!(exception.error_type, "budget_exceeded");
    assert_eq!(exception.param, None);
    assert_eq!(exception.code, "422");
}

#[test]
fn other_message_exception_is_a_401_auth_error_with_a_null_param() {
    let exception = other_message_exception("No api key passed in.");
    assert_eq!(
        exception.message,
        "Authentication Error, No api key passed in."
    );
    assert_eq!(exception.error_type, "auth_error");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert_eq!(exception.code, "401");
}

#[test]
fn model_denial_public_and_private_messages() {
    let failure = as_proxy_exception(&AuthFailure::ModelAccessDenied {
        detail: "key not allowed to access model".to_owned(),
        internal_message: "team grants: [a]\nkey grants: [b]".to_owned(),
        status_code: 403,
    });
    assert_eq!(
        wire(&failure),
        (
            "auth_error".to_owned(),
            Some("None".to_owned()),
            "403".to_owned()
        )
    );
    assert_eq!(
        failure.exception().message,
        "key not allowed to access model"
    );
    let ClientFailure::ModelAccessDenied(denial) = failure else {
        panic!("a model denial keeps its private message");
    };
    assert_eq!(
        denial.sanitized_internal_message(),
        "team grants: [a]key grants: [b]"
    );
}

#[rstest]
#[case::declared_param(Some("key"), 400, "key", "400")]
#[case::absent_param(None, 403, "None", "403")]
fn http_detail_param_and_status(
    #[case] param: Option<&str>,
    #[case] status_code: u16,
    #[case] wire_param: &str,
    #[case] code: &str,
) {
    let failure = as_proxy_exception(&AuthFailure::Http {
        detail: "Malformed API Key passed in.".to_owned(),
        param: param.map(str::to_owned),
        status_code,
        invalid_virtual_key: false,
    });
    assert_eq!(
        wire(&failure),
        (
            "auth_error".to_owned(),
            Some(wire_param.to_owned()),
            code.to_owned()
        )
    );
    assert_eq!(failure.exception().message, "Malformed API Key passed in.");
    assert!(matches!(failure, ClientFailure::Proxy(_)));
}

#[test]
fn proxy_exception_unchanged() {
    let exception = ProxyException::new(ProxyExceptionInit {
        message: "No connected db.".to_owned(),
        error_type: "no_db_connection".to_owned(),
        param: None,
        code: ExceptionCode::Status(400),
        headers: BTreeMap::from([("retry-after".to_owned(), "5".to_owned())]),
        openai_code: Some("db_down".to_owned()),
        provider_specific_fields: BTreeMap::new(),
    });
    let failure = as_proxy_exception(&AuthFailure::Proxy {
        exception: exception.clone(),
        invalid_virtual_key: false,
    });
    assert_eq!(failure, ClientFailure::Proxy(exception));
}

#[test]
fn database_service_unavailable_503() {
    let outage = as_proxy_exception(&database(
        ErrorType::Connect,
        "Can't reach database server at localhost:5432",
    ));
    assert_eq!(
        wire(&outage),
        (
            "no_db_connection".to_owned(),
            Some("None".to_owned()),
            "503".to_owned()
        )
    );
    let data = as_proxy_exception(&database(ErrorType::UniqueViolation, "duplicate key"));
    assert_eq!(
        wire(&data),
        (
            "auth_error".to_owned(),
            Some("None".to_owned()),
            "401".to_owned()
        )
    );
    assert_eq!(
        data.exception().message,
        "Authentication Error, duplicate key"
    );
}

#[rstest]
#[case::no_param(None, "None")]
#[case::declared_param(Some("model"), "model")]
fn ordinary_error_401_prefix(#[case] param: Option<&str>, #[case] wire_param: &str) {
    let failure = as_proxy_exception(&AuthFailure::Other {
        message: "No api key passed in.".to_owned(),
        param: param.map(str::to_owned),
    });
    assert_eq!(
        wire(&failure),
        (
            "auth_error".to_owned(),
            Some(wire_param.to_owned()),
            "401".to_owned()
        )
    );
    assert_eq!(
        failure.exception().message,
        "Authentication Error, No api key passed in."
    );
}

#[test]
fn client_context_key_presence_falsey_fields_and_copy() {
    let preferred = object(&[
        ("litellm_metadata", Json::Null),
        (
            "metadata",
            Json::Object(object(&[("user_agent", Json::Text("old".to_owned()))])),
        ),
    ]);
    let stamped = with_client_context(&preferred, Some(CLIENT_IP), Some("curl/8.5"));
    let expected = object(&[
        ("requester_ip_address", Json::Text(CLIENT_IP.to_owned())),
        ("user_agent", Json::Text("curl/8.5".to_owned())),
    ]);
    assert_eq!(
        stamped.get("litellm_metadata"),
        Some(&Json::Object(expected))
    );
    assert_eq!(stamped.get("metadata"), preferred.get("metadata"));
    assert_eq!(preferred.get("litellm_metadata"), Some(&Json::Null));
    let partly = object(&[(
        "metadata",
        Json::Object(object(&[
            ("user_agent", Json::Text(String::new())),
            ("requester_ip_address", Json::Text("10.0.0.1".to_owned())),
        ])),
    )]);
    let filled = with_client_context(&partly, Some(CLIENT_IP), Some("curl/8.5"));
    let merged = object(&[
        ("user_agent", Json::Text("curl/8.5".to_owned())),
        ("requester_ip_address", Json::Text("10.0.0.1".to_owned())),
    ]);
    assert_eq!(filled.get("metadata"), Some(&Json::Object(merged)));
    let bare = object(&[("model", Json::Text("gpt-4o".to_owned()))]);
    let untouched = with_client_context(&bare, Some(""), None);
    let Cow::Borrowed(borrowed) = untouched else {
        panic!("an unstamped request data stays borrowed");
    };
    assert!(ptr::eq(ptr::from_ref(borrowed), ptr::from_ref(&bare)));
    let owned = with_client_context(&bare, Some(CLIENT_IP), None);
    assert!(matches!(owned, Cow::Owned(_)));
    assert_eq!(bare.len(), 1);
}

#[test]
fn identity_suffix_order_and_unicode_escaping() {
    let identity = identity_with(&[
        ("team_alias", "ops\u{2028}team"),
        ("team_id", ""),
        ("user_email", "a\tb@example.com"),
        ("user_id", "user\u{1b}[31m"),
        ("key_alias", "alias \u{e000}\u{10ffff}\u{a0}\\"),
    ]);
    assert_eq!(
        identity_log_suffix(Some(&identity), true),
        "\nKey Identity: key_alias=alias \\ue000\\U0010ffff\\xa0\\ user_id=user\\x1b[31m \
         user_email=a\\tb@example.com team_alias=ops\\u2028team"
    );
    assert_eq!(identity_log_suffix(Some(&identity), false), "");
    assert_eq!(identity_log_suffix(None, true), "");
    assert_eq!(
        identity_log_suffix(Some(&identity_with(&[("team_id", "")])), true),
        ""
    );
    assert_eq!(
        escape_control_chars("line\nbreak\r\u{7f}"),
        "line\\nbreak\\r\\x7f"
    );
}
