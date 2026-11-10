use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_http::proxy::outbound::RequestBody;
use http::Method;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::workload_identity::error::WorkloadIdentityError;
use crate::workload_identity::exchange::{
    TOKEN_EXCHANGE_URL, decode_token_exchange, exchange_request,
};

#[derive(Deserialize)]
struct Body {
    grant_type: String,
    subject_token: String,
    subject_token_type: String,
    identity_provider_id: String,
    service_account_id: String,
    #[serde(flatten)]
    rest: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq)]
enum Decoded {
    Token(&'static str, f64),
    Empty,
    Invalid(&'static str),
}

fn decoded(bytes: &[u8]) -> Decoded {
    match decode_token_exchange(bytes) {
        Ok(exchange) => Decoded::Token(
            match exchange.access_token.expose_secret() {
                "abc" => "abc",
                "t" => "t",
                _other => "unexpected",
            },
            exchange.expires_in,
        ),
        Err(WorkloadIdentityError::EmptyBody) => Decoded::Empty,
        Err(WorkloadIdentityError::InvalidField { field }) => Decoded::Invalid(field),
        Err(other) => panic!("unexpected failure {other:?}"),
    }
}

pub(super) fn exchange_json_fields() {
    accepted_replies();
    rejected_fields();
    rejected_documents();
    request_fields();
}

fn accepted_replies() {
    let table = [
        (
            r#"{"access_token":"abc","expires_in":3600}"#,
            Decoded::Token("abc", 3600.0),
        ),
        (
            r#"{"access_token":"abc","expires_in":3600.5}"#,
            Decoded::Token("abc", 3600.5),
        ),
        (
            r#"{"access_token":"abc","expires_in":0}"#,
            Decoded::Token("abc", 0.0),
        ),
        (
            r#"{"access_token":"abc","expires_in":-5}"#,
            Decoded::Token("abc", -5.0),
        ),
        (
            r#"{"access_token":"t","expires_in":1,"token_type":"Bearer","scope":null}"#,
            Decoded::Token("t", 1.0),
        ),
    ];
    for (body, expected) in table {
        assert_eq!(decoded(body.as_bytes()), expected, "{body}");
    }
}

fn rejected_fields() {
    let table = [
        (r#"{"expires_in":3600}"#, "access_token"),
        (r#"{"access_token":"","expires_in":3600}"#, "access_token"),
        (r#"{"access_token":null,"expires_in":3600}"#, "access_token"),
        (r#"{"access_token":7,"expires_in":3600}"#, "access_token"),
        (
            r#"{"access_token":["abc"],"expires_in":3600}"#,
            "access_token",
        ),
        ("{}", "access_token"),
        (r#"{"access_token":"abc"}"#, "expires_in"),
        (r#"{"access_token":"abc","expires_in":null}"#, "expires_in"),
        (
            r#"{"access_token":"abc","expires_in":"3600"}"#,
            "expires_in",
        ),
        (r#"{"access_token":"abc","expires_in":true}"#, "expires_in"),
    ];
    for (body, field) in table {
        assert_eq!(decoded(body.as_bytes()), Decoded::Invalid(field), "{body}");
    }
}

fn rejected_documents() {
    for body in ["[]", "[1]", r#""text""#, "7"] {
        assert_eq!(
            decoded(body.as_bytes()),
            Decoded::Invalid("access_token"),
            "{body}"
        );
    }
    let empty = [
        "null",
        "",
        "   ",
        "not json",
        r#"{"access_token":"abc","expires_in":36"#,
    ];
    for body in empty {
        assert_eq!(decoded(body.as_bytes()), Decoded::Empty, "{body}");
    }
    assert_eq!(decoded(&[0xff, 0xfe]), Decoded::Empty);
}

fn request_fields() {
    let subject = SecretString::from("jwt".to_owned());
    let request = exchange_request(TOKEN_EXCHANGE_URL, &subject, "idp", "sa").unwrap();
    let RequestBody::Json(bytes) = &request.body else {
        panic!("the exchange body is JSON");
    };
    let body: Body = serde_json::from_slice(bytes).unwrap();
    assert_eq!(
        body.grant_type,
        "urn:ietf:params:oauth:grant-type:token-exchange"
    );
    assert_eq!(body.subject_token, "jwt");
    assert_eq!(
        body.subject_token_type,
        "urn:ietf:params:oauth:token-type:jwt"
    );
    assert_eq!(body.identity_provider_id, "idp");
    assert_eq!(body.service_account_id, "sa");
    assert!(body.rest.is_empty(), "{:?}", body.rest);
    assert_eq!(request.url.as_str(), "https://auth.openai.com/oauth/token");
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.deadline, Duration::from_secs(10));
    assert!(request.headers.is_empty());
    assert!(request.query.is_empty());
    let invalid = exchange_request("not a url", &subject, "idp", "sa");
    assert!(matches!(
        invalid,
        Err(WorkloadIdentityError::ExchangeUrl(_))
    ));
}
