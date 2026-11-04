use super::{
    HttpResponse, RevocationUnavailableOrFailure, ScriptedHttp, contract, form_fields,
    revoke_credential,
};
use secrecy::SecretString;

#[test]
fn revoke_posts_an_rfc7009_request_for_the_refresh_token() {
    let response = HttpResponse {
        status: 200,
        headers: Vec::new(),
        body: Vec::new(),
    };
    let client = ScriptedHttp::new(response);
    revoke_credential(
        &contract(),
        "client",
        &SecretString::from("refresh"),
        &client,
    )
    .unwrap();
    let request = client.requests().pop().expect("request");
    assert_eq!(request.url, "https://example.test/oauth/revoke");
    assert_eq!(
        form_fields(&request.body),
        vec![
            ("token".to_owned(), "refresh".to_owned()),
            ("token_type_hint".to_owned(), "refresh_token".to_owned()),
            ("client_id".to_owned(), "client".to_owned())
        ]
    );
}

#[rstest::rstest]
#[case::invalid_client(401, br#"{"error":"invalid_client"}"#, "invalid_client")]
#[case::server_error(500, b"server error", "server error")]
fn revoke_non_503_errors_are_failures(
    #[case] status: u16,
    #[case] body: &[u8],
    #[case] detail: &str,
) {
    let client = ScriptedHttp::new(HttpResponse {
        status,
        headers: Vec::new(),
        body: body.to_vec(),
    });
    let error = revoke_credential(
        &contract(),
        "client",
        &SecretString::from("refresh"),
        &client,
    )
    .expect_err("revocation failed");
    let RevocationUnavailableOrFailure::Failure(failure) = error else {
        panic!("expected a revocation failure");
    };
    assert!(failure.to_string().contains(detail));
}
