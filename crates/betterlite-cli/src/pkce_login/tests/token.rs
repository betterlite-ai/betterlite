use super::*;

fn token_client() -> ScriptedHttp {
    ScriptedHttp::new(HttpResponse { status: 200, headers: Vec::new(), body: br#"{"access_token":"sk-cli-new","refresh_token":"llm_srefresh_new","expires_in":3600,"user_id":"u1","team_id":"team-b"}"#.to_vec() })
}

fn assert_credential(client: &ScriptedHttp, credential: &super::super::models::PkceCredential) {
    assert_eq!(credential.access_token.expose_secret(), "sk-cli-new");
    assert_eq!(credential.refresh_token.expose_secret(), "llm_srefresh_new");
    assert_eq!(credential.client_id, "client");
    assert_eq!(
        credential.token_endpoint,
        "https://example.test/oauth/token"
    );
    assert_eq!(
        credential.revocation_endpoint,
        "https://example.test/oauth/revoke"
    );
    assert_eq!(credential.resource, "https://example.test");
    assert_eq!(credential.user_id.as_deref(), Some("u1"));
    assert_eq!(credential.team_id.as_deref(), Some("team-b"));
    assert_eq!(client.requests()[0].url, "https://example.test/oauth/token");
}

#[test]
fn token_redeem_binds_the_credential_to_the_contract() {
    let client = token_client();
    let credential =
        redeem_code(&contract(), &redemption(), &client, &FixedWallClock(100)).expect("credential");
    assert_credential(&client, &credential);
}

#[test]
fn token_refresh_binds_the_credential_to_the_contract() {
    let client = token_client();
    let request = RefreshCredentialRequest {
        client_id: "client".to_owned(),
        refresh_token: SecretString::from("old"),
    };
    let credential = refresh_credential(&contract(), &request, &client, &FixedWallClock(100))
        .expect("credential");
    assert_credential(&client, &credential);
}
