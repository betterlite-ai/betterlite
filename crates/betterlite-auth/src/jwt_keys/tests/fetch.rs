use std::time::Duration;

use betterlite_models::decisions::Json;
use tracing::Level;

use crate::jwt_keys::errors::{KeyError, exception_for};
use crate::jwt_keys::resolver::{DISCOVERY_CACHE_KEY_PREFIX, KEYS_CACHE_KEY_PREFIX};

use super::support::{
    DISCOVERY_URL, JWKS_URL, KEYS_KEY, LevelCapture, Reply, ScriptedHttp, fixture, json, key, keys,
    outage, settings,
};

#[tokio::test]
async fn configured_jwks() {
    assert_eq!(KEYS_KEY, format!("{KEYS_CACHE_KEY_PREFIX}{JWKS_URL}"));
    let fixture = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));
    assert_eq!(fixture.http.requests(), vec![JWKS_URL.to_owned()]);
    let writes = fixture.cache.writes();
    assert_eq!(writes[0].key, KEYS_KEY);
    assert_eq!(writes[0].value, keys(&[key("a")]));
    assert!(writes[0].ttl.total_cmp(&600.0_f64).is_eq());
}

#[tokio::test]
async fn oidc_discovery() {
    let discovery = format!("{DISCOVERY_URL}?tenant=b");
    let jwks = "https://idp.example.com/tenant-b/jwks";
    let document = format!(r#"{{"issuer":"https://idp.example.com","jwks_uri":"{jwks}"}}"#);
    let fixture = fixture(
        settings(Some(&discovery), 600.0, 3600.0),
        1_000.0,
        vec![
            ScriptedHttp::text(200, &document),
            ScriptedHttp::answer(&keys(&[key("a")])),
        ],
    );
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));
    assert_eq!(
        fixture.http.requests(),
        vec![discovery.clone(), jwks.to_owned()]
    );
    let writes = fixture.cache.writes();
    assert_eq!(
        writes[0].key,
        format!("{DISCOVERY_CACHE_KEY_PREFIX}{discovery}")
    );
    assert_eq!(writes[0].value, Json::Text(jwks.to_owned()));
    assert_eq!(writes[3].key, format!("{KEYS_CACHE_KEY_PREFIX}{jwks}"));
    assert_eq!(writes[3].value, keys(&[key("a")]));

    // both the discovery lookup and the keys are cached, so the next call goes nowhere
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, keys(&[key("a")]));
    assert_eq!(fixture.http.requests().len(), 2);
}

#[tokio::test]
async fn discovery_status_parse_missing_uri_errors() {
    // a status is a discovery failure with the body in the message
    let status = fixture(
        settings(Some(DISCOVERY_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(404, "not found")],
    );
    let error = status.resolver.get_public_key(None).await.unwrap_err();
    assert!(matches!(
        error,
        KeyError::DiscoveryStatus {
            status: 404,
            ref body,
            ..
        } if body == "not found"
    ));
    assert_eq!(status.http.requests().len(), 1);

    // invalid JSON, a non-object document, and every unusable jwks_uri shape have their own failures
    for body in [
        "{",
        "[]",
        "{}",
        r#"{"jwks_uri":""}"#,
        r#"{"jwks_uri":null}"#,
        r#"{"jwks_uri":7}"#,
        r#"{"jwks_uri":["https://idp.example.com/jwks"]}"#,
    ] {
        let fixture = fixture(
            settings(Some(DISCOVERY_URL), 600.0, 3600.0),
            1_000.0,
            vec![ScriptedHttp::text(200, body)],
        );
        let error = fixture
            .resolver
            .get_public_key(Some("a"))
            .await
            .unwrap_err();
        let expected = if body == "{" {
            matches!(error, KeyError::DiscoveryParse { .. })
        } else {
            matches!(error, KeyError::DiscoveryMissingUri { .. })
        };
        assert!(expected, "body {body} gave {error:?}");
        assert!(fixture.cache.writes().is_empty());
    }
}

#[tokio::test(flavor = "current_thread")]
async fn jwks_status_parse_log_and_keys_envelope() {
    // a JWKS status names the status and the body
    let status = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(500, "upstream broke")],
    );
    let error = status.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(
        error,
        KeyError::JwksStatus {
            status: 500,
            ref body,
            ..
        } if body == "upstream broke"
    ));

    // a parse failure logs the body at ERROR and raises the short message
    let malformed = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, "{\"keys\":[")],
    );
    let capture = LevelCapture::new();
    let error = malformed
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::JwksParse { .. }));
    let levels = capture.levels();
    assert_eq!(
        levels
            .iter()
            .filter(|level| **level == Level::ERROR)
            .count(),
        1
    );
    let messages = capture.messages();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Error parsing response")
                && message.contains("Original Response: {\"keys\":[")),
        "the parse failure logs the original body: {messages:?}"
    );
    drop(capture);

    // the keys member is what gets cached when it is there
    let envelope = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(
            200,
            r#"{"issuer":"x","keys":[{"kid":"a","kty":"RSA","n":"one"}]}"#,
        )],
    );
    let value = envelope.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, json(r#"{"kid":"a","kty":"RSA","n":"one"}"#));
    assert_eq!(
        envelope.cache.writes()[0].value,
        keys(&[json(r#"{"kid":"a","kty":"RSA","n":"one"}"#)])
    );

    // without a keys member the response itself is the key material, list or object
    let bare_list = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, r#"[{"kid":"a","n":"one"}]"#)],
    );
    let value = bare_list.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, json(r#"{"kid":"a","n":"one"}"#));

    let bare_key = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, r#"{"kid":"a"}"#)],
    );
    let value = bare_key.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, json(r#"{"kid":"a"}"#));
}

#[tokio::test]
async fn malformed_jwks() {
    let fixture = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, "not json")],
    );
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::JwksParse { .. }));
    assert_eq!(
        error.to_string(),
        "Error parsing response: the json document is not valid json. Check server logs for original response."
    );
    assert_eq!(fixture.http.requests().len(), 1);
    assert!(fixture.cache.writes().is_empty());

    // a scalar body reaches parse_keys instead, which finds no key at all
    let scalar = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, "7")],
    );
    let error = scalar.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::NoMatchingKey { .. }));
}

#[tokio::test]
async fn transport_three_attempts() {
    let fixture = fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert_eq!(fixture.http.requests(), vec![JWKS_URL.to_owned(); 3]);
    assert_eq!(
        fixture.clock.sleeps(),
        vec![Duration::from_millis(250), Duration::from_millis(500)]
    );
    assert!(matches!(
        error,
        KeyError::Unreachable { ref message }
            if message.contains("after 3 attempts") && message.contains(JWKS_URL)
    ));
}

#[tokio::test]
async fn http_error_no_transport_retry() {
    // a status is not retried
    let status = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(503, "busy")],
    );
    let error = status.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(
        error,
        KeyError::JwksStatus {
            status: 503,
            ref body,
            ..
        } if body == "busy"
    ));
    assert_eq!(status.http.requests().len(), 1);
    assert!(status.clock.sleeps().is_empty());

    // a port-level failure is not retried either
    let failed = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![Reply::Failed],
    );
    let error = failed.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error, KeyError::Fetch { .. }));
    assert_eq!(failed.http.requests().len(), 1);
    assert!(failed.clock.sleeps().is_empty());
}

#[tokio::test]
async fn absent_empty_and_ordered_url_list() {
    // absent is a hard failure before any fetch
    let absent = fixture(settings(None, 600.0, 3600.0), 1_000.0, Vec::new());
    let error = absent.resolver.get_public_key(None).await.unwrap_err();
    assert!(matches!(error, KeyError::MissingUrl));
    assert_eq!(
        error.to_string(),
        "Missing JWT Public Key URL from environment."
    );
    assert!(absent.http.requests().is_empty());

    // empty is an empty list, which is no URL to try
    for raw in ["", "   ", " , , "] {
        let empty = fixture(settings(Some(raw), 600.0, 3600.0), 1_000.0, Vec::new());
        let error = empty.resolver.get_public_key(None).await.unwrap_err();
        assert!(matches!(
            error,
            KeyError::NoMatchingKey { ref keys, kid: None } if keys == "[]"
        ));
        assert_eq!(
            error.to_string(),
            "No matching public key found. keys=[], kid=None"
        );
        assert!(empty.http.requests().is_empty());
    }

    // the list is tried in order, each entry trimmed
    let first = "https://a.example.com/keys";
    let second = "https://b.example.com/keys";
    let raw = " https://b.example.com/keys , , https://a.example.com/keys ";
    let ordered = fixture(
        settings(Some(raw), 600.0, 3600.0),
        1_000.0,
        vec![
            ScriptedHttp::answer(&keys(&[key("b")])),
            ScriptedHttp::answer(&keys(&[key("a")])),
        ],
    );
    let value = ordered.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
    assert_eq!(
        ordered.http.requests(),
        vec![second.to_owned(), first.to_owned()]
    );

    // an entry that was already fetched and has no match is not fetched again
    let repeat = format!("{first},{first}");
    let duplicate = fixture(
        settings(Some(&repeat), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("b")]))],
    );
    let error = duplicate
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert_eq!(duplicate.http.requests(), vec![first.to_owned()]);
    assert_eq!(
        error.to_string(),
        "No matching public key found. keys=['https://a.example.com/keys', 'https://a.example.com/keys'], kid=a"
    );
}

#[tokio::test]
async fn first_outage_stops_url_iteration_with_503_fields() {
    let first = "https://a.example.com/keys";
    let raw = format!("{first},https://b.example.com/keys");
    let fixture = fixture(settings(Some(&raw), 600.0, 3600.0), 1_000.0, outage());
    let error = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    assert_eq!(fixture.http.requests(), vec![first.to_owned(); 3]);
    let exception = exception_for(&error);
    assert_eq!(exception.code, "503");
    assert_eq!(exception.error_type, "auth_provider_unavailable");
    assert_eq!(exception.param.as_deref(), Some("None"));
    assert!(exception.message.starts_with(
        "Service Unavailable, the identity provider's JWKS endpoint is temporarily unreachable, \
         so the JWT signature could not be verified. Please retry shortly. Error: "
    ));
    assert!(exception.message.contains(first));
    assert!(exception.message.contains("after 3 attempts"));
}

#[tokio::test]
async fn kid_mismatch() {
    let fixture = fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    let error = fixture
        .resolver
        .get_public_key(Some("b"))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        KeyError::NoMatchingKey { ref keys, ref kid }
            if keys == JWKS_URL && kid.as_deref() == Some("b")
    ));
    assert_eq!(
        error.to_string(),
        "No matching public key found. keys=https://idp.example.com/jwks, kid=b"
    );

    // the fetch succeeded, so the keys are cached for the next caller
    let error = fixture
        .resolver
        .get_public_key(Some("b"))
        .await
        .unwrap_err();
    assert!(matches!(error, KeyError::NoMatchingKey { .. }));
    assert_eq!(fixture.http.requests().len(), 1);
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
    assert_eq!(fixture.http.requests().len(), 1);
}

#[tokio::test]
async fn kid_miss_continues_without_refresh() {
    let first = "https://one.example.com/jwks";
    let second = "https://two.example.com/jwks";
    let raw = format!("{first}, {second}");
    let fixture = fixture(
        settings(Some(&raw), 600.0, 3600.0),
        1_000.0,
        vec![
            ScriptedHttp::answer(&keys(&[key("b")])),
            ScriptedHttp::answer(&keys(&[key("a")])),
        ],
    );

    // the first endpoint misses the kid and the second answers it
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
    assert_eq!(
        fixture.http.requests(),
        vec![first.to_owned(), second.to_owned()]
    );

    // both fetches were cached, so the same kid is served without refreshing either
    let again = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(again, key("a"));
    let missed = fixture
        .resolver
        .get_public_key(Some("absent"))
        .await
        .unwrap_err();
    assert!(matches!(missed, KeyError::NoMatchingKey { .. }));
    assert_eq!(fixture.http.requests().len(), 2);
}
