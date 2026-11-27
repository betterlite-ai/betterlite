use super::support::{JWKS_URL, ScriptedHttp, key, keys, new_resolver_fixture, settings};

#[tokio::test(flavor = "current_thread")]
async fn concurrent_refresh_single_flight() {
    let fixture = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    let (one, two, three) = tokio::join!(
        fixture.resolver.get_public_key(Some("a")),
        fixture.resolver.get_public_key(Some("a")),
        fixture.resolver.get_public_key(Some("a")),
    );
    assert_eq!(one.unwrap(), key("a"));
    assert_eq!(two.unwrap(), key("a"));
    assert_eq!(three.unwrap(), key("a"));

    // one Refresh went to the IdP; the other two waited on the lock and read what the first wrote
    assert_eq!(fixture.http.requests(), vec![JWKS_URL.to_owned()]);
    assert_eq!(fixture.cache.writes().len(), 3);

    // and the next caller does not touch the IdP at all
    fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(fixture.http.requests().len(), 1);
}
