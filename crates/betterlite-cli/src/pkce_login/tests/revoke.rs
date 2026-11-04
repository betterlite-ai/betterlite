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

fn assert_revoke_failure(status: u16, body: Vec<u8>) {
    let client = ScriptedHttp::new(HttpResponse {
        status,
        headers: Vec::new(),
        body,
    });
    assert!(matches!(
        revoke_credential(
            &contract(),
            "client",
            &SecretString::from("refresh"),
            &client
        ),
        Err(RevocationUnavailableOrFailure::Failure(_))
    ));
}

#[test]
fn revoke_non_503_errors_are_failures() {
    assert_revoke_failure(401, br#"{"error":"invalid_client"}"#.to_vec());
    assert_revoke_failure(500, Vec::new());
}
