use betterlite_models::decisions::Json;

use crate::jwt_keys::resolver::{
    DISCOVERY_CACHE_KEY_PREFIX, KEYS_CACHE_KEY_PREFIX, STALE_CACHE_KEY_PREFIX,
    STALE_WRITTEN_AT_PREFIX,
};

use super::support::{
    DISCOVERY_URL, JWKS_URL, ScriptedHttp, key, keys, new_resolver_fixture, settings,
};

#[tokio::test]
async fn plain_jwks_url_needs_no_discovery() {
    // a plain JWKS URL is returned untouched, with no fetch
    let plain = new_resolver_fixture(settings(None, 600.0, 3600.0), 1_000.0, Vec::new());
    assert_eq!(
        plain.resolver.resolve_jwks_url(JWKS_URL).await.unwrap(),
        JWKS_URL
    );
    assert!(plain.http.requests().is_empty());
    assert!(plain.cache.writes().is_empty());
}

#[tokio::test]
async fn discovery_substring_detection_and_exact_cache_keys() {
    // the marker is a substring test, so a query string after it still counts as a discovery document
    let discovery = format!("{DISCOVERY_URL}?tenant=a");
    let jwks = "https://idp.example.com/tenant-a/jwks";
    let document = format!(r#"{{"jwks_uri":"{jwks}"}}"#);
    let looked_up = new_resolver_fixture(
        settings(None, 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &document)],
    );
    let resolved = looked_up
        .resolver
        .resolve_jwks_url(&discovery)
        .await
        .unwrap();
    assert_eq!(resolved, jwks);
    assert_eq!(looked_up.http.requests(), vec![discovery.clone()]);
    let writes = looked_up.cache.writes();
    let cache_key = format!("{DISCOVERY_CACHE_KEY_PREFIX}{discovery}");
    assert_eq!(writes[0].key, cache_key);
    assert_eq!(writes[0].value, Json::Text(jwks.to_owned()));
    assert!(writes[0].ttl.total_cmp(&600.0_f64).is_eq());
    assert_eq!(
        writes[1].key,
        format!("{STALE_CACHE_KEY_PREFIX}{cache_key}")
    );
    assert_eq!(
        writes[2].key,
        format!("{STALE_WRITTEN_AT_PREFIX}{cache_key}")
    );

    // the second lookup is served from the cache, not the IdP
    let again = looked_up
        .resolver
        .resolve_jwks_url(&discovery)
        .await
        .unwrap();
    assert_eq!(again, jwks);
    assert_eq!(looked_up.http.requests().len(), 1);
}

#[tokio::test]
async fn discovery_marker_is_substring() {
    let jwks = "https://idp.example.com/tenant-a/jwks";
    let document = format!(r#"{{"jwks_uri":"{jwks}"}}"#);
    // the marker anywhere in the URL counts, including inside a query value
    let in_query = "https://idp.example.com/jwks?next=/.well-known/openid-configuration";
    let elsewhere = new_resolver_fixture(
        settings(None, 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, &document)],
    );
    let resolved = elsewhere.resolver.resolve_jwks_url(in_query).await.unwrap();
    assert_eq!(resolved, jwks);
    assert_eq!(elsewhere.http.requests(), vec![in_query.to_owned()]);

    // the detection is case sensitive
    let upper = "https://idp.example.com/.WELL-KNOWN/openid-configuration";
    let untouched = new_resolver_fixture(settings(None, 600.0, 3600.0), 1_000.0, Vec::new());
    assert_eq!(
        untouched.resolver.resolve_jwks_url(upper).await.unwrap(),
        upper
    );
    assert!(untouched.http.requests().is_empty());
}

#[tokio::test]
async fn keys_cache_key_is_the_resolved_url() {
    let discovery = format!("{DISCOVERY_URL}?realm=a");
    let jwks = "https://idp.example.com/realms/a/jwks?use=signing";
    let document = format!(r#"{{"jwks_uri":"{jwks}"}}"#);
    let fixture = new_resolver_fixture(
        settings(Some(&discovery), 600.0, 3600.0),
        1_000.0,
        vec![
            ScriptedHttp::text(200, &document),
            ScriptedHttp::answer(&keys(&[key("a")])),
        ],
    );
    fixture.resolver.get_public_key(Some("a")).await.unwrap();
    let writes = fixture.cache.writes();
    assert_eq!(
        writes[3].key,
        format!("{KEYS_CACHE_KEY_PREFIX}{jwks}"),
        "the keys cache key is built from the discovery result, query string included"
    );
}
