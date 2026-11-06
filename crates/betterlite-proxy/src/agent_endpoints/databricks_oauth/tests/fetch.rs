#![cfg(test)]

use std::time::Duration;

use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{CLOSED, client, config, upstream};
use crate::agent_endpoints::databricks_oauth::fetch::fetch_token;

const STATUS_401: &str = "Databricks App OAuth token request failed with status 401";
const MISSING_TOKEN: &str = "Databricks App OAuth token response missing 'access_token'";
const NOT_OBJECT: &str = "Databricks App OAuth token response is not a JSON object";

// A status of 0 sends the request to a closed port instead of the local upstream.
#[rstest]
#[case::success(200, r#"{"access_token":"tok","expires_in":3600}"#, "tok|3540")]
#[case::success_201(201, r#"{"access_token":"tok","expires_in":3600}"#, "tok|3540")]
#[case::missing_expires_in(200, r#"{"access_token":"tok"}"#, "tok|3540")]
#[case::null_expires_in(200, r#"{"access_token":"tok","expires_in":null}"#, "tok|3540")]
#[case::numeric_string_expires_in(200, r#"{"access_token":"tok","expires_in":"120"}"#, "tok|60")]
#[case::padded_numeric_string_expires_in(
    200,
    r#"{"access_token":"tok","expires_in":" +120 "}"#,
    "tok|60"
)]
#[case::underscored_string_expires_in(
    200,
    r#"{"access_token":"tok","expires_in":"1_200"}"#,
    "tok|1140"
)]
#[case::float_expires_in(200, r#"{"access_token":"tok","expires_in":120.9}"#, "tok|60")]
#[case::boolean_expires_in(200, r#"{"access_token":"tok","expires_in":true}"#, "tok|0")]
#[case::non_numeric_string_expires_in(
    200,
    r#"{"access_token":"tok","expires_in":"abc"}"#,
    "tok|3540"
)]
#[case::empty_string_expires_in(200, r#"{"access_token":"tok","expires_in":""}"#, "tok|3540")]
#[case::double_underscore_expires_in(
    200,
    r#"{"access_token":"tok","expires_in":"1__2"}"#,
    "tok|3540"
)]
#[case::double_sign_expires_in(200, r#"{"access_token":"tok","expires_in":"--5"}"#, "tok|3540")]
#[case::object_expires_in(200, r#"{"access_token":"tok","expires_in":{}}"#, "tok|3540")]
#[case::list_expires_in(200, r#"{"access_token":"tok","expires_in":[]}"#, "tok|3540")]
#[case::lifetime_below_buffer(200, r#"{"access_token":"tok","expires_in":30}"#, "tok|0")]
#[case::lifetime_equal_to_buffer(200, r#"{"access_token":"tok","expires_in":60}"#, "tok|0")]
#[case::lifetime_just_above_buffer(200, r#"{"access_token":"tok","expires_in":61}"#, "tok|1")]
#[case::negative_expires_in(200, r#"{"access_token":"tok","expires_in":-5}"#, "tok|0")]
#[case::huge_expires_in_is_capped(
    200,
    r#"{"access_token":"tok","expires_in":99999999999999999999}"#,
    "tok|4294967295"
)]
#[case::http_error_401(401, r#"{"error":"invalid_client"}"#, STATUS_401)]
#[case::http_error_500(500, "", "Databricks App OAuth token request failed with status 500")]
#[case::http_redirect_is_an_error(
    302,
    "",
    "Databricks App OAuth token request failed with status 302"
)]
#[case::missing_access_token(200, r#"{"expires_in":3600}"#, MISSING_TOKEN)]
#[case::empty_access_token(200, r#"{"access_token":""}"#, MISSING_TOKEN)]
#[case::null_access_token(200, r#"{"access_token":null}"#, MISSING_TOKEN)]
#[case::non_string_access_token(200, r#"{"access_token":5}"#, MISSING_TOKEN)]
#[case::non_dict_response_list(200, r#"["tok",3600]"#, NOT_OBJECT)]
#[case::non_dict_response_string(200, r#""tok""#, NOT_OBJECT)]
#[case::non_dict_response_not_json(200, "not json", NOT_OBJECT)]
#[case::other_transport_error(0, "", "Databricks App OAuth token request failed")]
#[tokio::test]
async fn databricks_oauth_tests_behavior(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: &str,
) {
    let server = upstream(status, body, Duration::ZERO).await;
    let url = if status == 0 { CLOSED } else { &server.url };
    let observed = match fetch_token(&client(), &config(url, "secret")).await {
        Ok((token, ttl)) => format!("{}|{ttl}", token.expose_secret()),
        Err(error) => error.to_string(),
    };
    assert_eq!(observed, expected);
    let sent = server.requests.lock().unwrap().join("");
    if status == 0 {
        assert_eq!(sent, "");
    } else {
        assert!(
            sent.starts_with("POST /oidc/v1/token HTTP/1.1\r\n"),
            "{sent}"
        );
        assert!(
            sent.contains("authorization: Basic aWQ6c2VjcmV0\r\n"),
            "{sent}"
        );
        assert!(
            sent.contains("content-type: application/x-www-form-urlencoded\r\n"),
            "{sent}"
        );
        assert!(
            sent.ends_with("\r\n\r\ngrant_type=client_credentials&scope=all-apis+sql"),
            "{sent}"
        );
    }
}
