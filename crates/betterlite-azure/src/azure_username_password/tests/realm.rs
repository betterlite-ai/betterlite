use super::fixture::{Fixture, HOST, PASSWORD, Reply, USERNAME, input_for, openid, token};
use super::scenarios::token_body;

fn realm_outcome(realm: Reply) -> (Result<(), String>, usize) {
    let fixture = Fixture::new(vec![openid(HOST), realm, token(&token_body())]);
    let outcome = fixture
        .provider(&input_for(USERNAME, PASSWORD, None))
        .unwrap()
        .token()
        .map(|_access| ())
        .map_err(|error| error.to_string());
    (outcome, fixture.transport.posts().len())
}

pub(super) fn managed_accounts_proceed() {
    for body in [
        r#"{"account_type":"Managed"}"#,
        "{}",
        r#"{"account_type":null}"#,
        r#"{"account_type":"federated"}"#,
    ] {
        assert_eq!(
            realm_outcome(Reply::ok("userrealm", body)),
            (Ok(()), 1),
            "{body}"
        );
    }
    assert_eq!(
        realm_outcome(Reply::status(404, "userrealm", "not found")),
        (Ok(()), 1)
    );
}

pub(super) fn federated_account_is_refused() {
    let (outcome, posts) = realm_outcome(Reply::ok(
        "userrealm",
        r#"{"account_type":"Federated","federation_protocol":"WSTrust"}"#,
    ));
    assert_eq!(
        outcome,
        Err(
            "Authentication failed: the account is federated, and the WS-Trust sign-in it needs is not supported"
                .to_owned()
        )
    );
    assert_eq!(posts, 0);
}

pub(super) fn realm_failures_stop_the_grant() {
    let cases = [
        (
            Reply::status(500, "userrealm", ""),
            "Authentication failed: HTTP Error: 500",
        ),
        (
            Reply::status(400, "userrealm", ""),
            "Authentication failed: HTTP Error: 400",
        ),
        (
            Reply::status(403, "userrealm", ""),
            "Authentication failed: HTTP Error: 403",
        ),
        (
            Reply::ok("userrealm", "<html>"),
            "the user realm response is not the expected JSON",
        ),
        (
            Reply::failure("userrealm"),
            "Authentication failed: connection reset",
        ),
    ];
    for (realm, message) in cases {
        assert_eq!(realm_outcome(realm), (Err(message.to_owned()), 0));
    }
}
