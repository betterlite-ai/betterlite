use std::time::Duration;

use betterlite_models::decisions::Json;
use rstest::rstest;
use tracing::Level;

use crate::jwt_keys::errors::{KeyError, exception_for};
use crate::jwt_keys::resolver::{DISCOVERY_CACHE_KEY_PREFIX, KEYS_CACHE_KEY_PREFIX};

use super::support::{
    DISCOVERY_URL, JWKS_URL, KEYS_KEY, LevelCapture, Reply, ScriptedHttp, json, key, keys,
    new_resolver_fixture, outage, settings,
};

#[tokio::test]
async fn configured_jwks() {
    assert_eq!(KEYS_KEY, format!("{KEYS_CACHE_KEY_PREFIX}{JWKS_URL}"));
    let fixture = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::answer(&keys(&[key("a")]))],
    );
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
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
    let fixture = new_resolver_fixture(
        settings(Some(&discovery), 600.0, 3600.0),
        1_000.0,
        vec![
            ScriptedHttp::text(200, &document),
            ScriptedHttp::answer(&keys(&[key("a")])),
        ],
    );
    let value = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value, key("a"));
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
    let value2 = fixture.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value2, key("a"));
    assert_eq!(fixture.http.requests().len(), 2);
}

#[rstest]
#[case::invalid_json("{")]
#[case::array("[]")]
#[case::empty_object("{}")]
#[case::empty_uri(r#"{"jwks_uri":""}"#)]
#[case::null_uri(r#"{"jwks_uri":null}"#)]
#[case::number_uri(r#"{"jwks_uri":7}"#)]
#[case::list_uri(r#"{"jwks_uri":["https://idp.example.com/jwks"]}"#)]
#[tokio::test]
async fn discovery_status_parse_missing_uri_errors(#[case] body: &str) {
    // a status is a discovery failure with the body in the message
    let status = new_resolver_fixture(
        settings(Some(DISCOVERY_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(404, "not found")],
    );
    let error = status.resolver.get_public_key(None).await.unwrap_err();
    assert!(matches!(
        &error,
        KeyError::DiscoveryStatus {
            status: 404,
            body: response_body,
            ..
        } if response_body == "not found"
    ));
    assert_eq!(status.http.requests().len(), 1);

    // invalid JSON, a non-object document, and every unusable jwks_uri shape have their own failures
    let fixture = new_resolver_fixture(
        settings(Some(DISCOVERY_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, body)],
    );
    let error2 = fixture
        .resolver
        .get_public_key(Some("a"))
        .await
        .unwrap_err();
    let expected = if body == "{" {
        matches!(error2, KeyError::DiscoveryParse { .. })
    } else {
        matches!(error2, KeyError::DiscoveryMissingUri { .. })
    };
    assert!(expected, "body {body} gave {error2:?}");
    assert!(fixture.cache.writes().is_empty());
}

#[rstest]
#[case::status(500_u16, "upstream broke", None)]
#[case::malformed(200_u16, "{", None)]
#[case::envelope(
    200_u16,
    r#"{"keys":[{"kid":"a","n":"one"}]}"#,
    Some(r#"{"kid":"a","n":"one"}"#)
)]
#[tokio::test]
async fn jwks_status_parse_log_and_keys_envelope(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Option<&str>,
) {
    let fixture = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(status, body)],
    );
    let capture = LevelCapture::new();
    let result = fixture.resolver.get_public_key(Some("a")).await;
    if let Some(selected) = expected {
        assert_eq!(result.unwrap(), json(selected));
        assert_eq!(fixture.cache.writes()[0].value, keys(&[json(selected)]));
    } else {
        let error = result.unwrap_err();
        if status == 500 {
            assert!(
                matches!(&error, KeyError::JwksStatus { status: 500, body: response, .. } if response == body)
            );
        } else {
            assert!(matches!(error, KeyError::JwksParse { .. }));
            assert!(capture.levels().contains(&Level::ERROR));
            assert!(
                capture
                    .messages()
                    .iter()
                    .any(|message| message.contains("Original Response: {"))
            );
        }
    }
}

#[tokio::test]
async fn jwks_parse_logs_response() {
    // a parse failure logs the body at ERROR and raises the short message
    let malformed = new_resolver_fixture(
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
}

#[tokio::test]
async fn jwks_envelope_and_bare_values() {
    // the keys member is what gets cached when it is there
    let envelope = new_resolver_fixture(
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
    let bare_list = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, r#"[{"kid":"a","n":"one"}]"#)],
    );
    let value2 = bare_list.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value2, json(r#"{"kid":"a","n":"one"}"#));

    let bare_key = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, r#"{"kid":"a"}"#)],
    );
    let value3 = bare_key.resolver.get_public_key(Some("a")).await.unwrap();
    assert_eq!(value3, json(r#"{"kid":"a"}"#));
}

#[tokio::test]
async fn malformed_jwks() {
    let fixture = new_resolver_fixture(
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
    let scalar = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(200, "7")],
    );
    let error2 = scalar.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error2, KeyError::NoMatchingKey { .. }));
}

#[tokio::test]
async fn transport_three_attempts() {
    let fixture = new_resolver_fixture(settings(Some(JWKS_URL), 600.0, 3600.0), 1_000.0, outage());
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
        &error,
        KeyError::Unreachable { message }
            if message.contains("after 3 attempts") && message.contains(JWKS_URL)
    ));
}

#[tokio::test]
async fn http_error_no_transport_retry() {
    // a status is not retried
    let status = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![ScriptedHttp::text(503, "busy")],
    );
    let error = status.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(
        &error,
        KeyError::JwksStatus {
            status: 503,
            body,
            ..
        } if body == "busy"
    ));
    assert_eq!(status.http.requests().len(), 1);
    assert!(status.clock.sleeps().is_empty());

    // a port-level failure is not retried either
    let failed = new_resolver_fixture(
        settings(Some(JWKS_URL), 600.0, 3600.0),
        1_000.0,
        vec![Reply::Failed],
    );
    let error2 = failed.resolver.get_public_key(Some("a")).await.unwrap_err();
    assert!(matches!(error2, KeyError::Fetch { .. }));
    assert_eq!(failed.http.requests().len(), 1);
    assert!(failed.clock.sleeps().is_empty());
}

#[rstest]
#[case::empty("", 0_usize)]
#[case::spaces("   ", 0_usize)]
#[case::commas(" , , ", 0_usize)]
#[case::ordered(" https://b.example.com/keys , , https://a.example.com/keys ", 2_usize)]
#[tokio::test]
async fn absent_empty_and_ordered_url_list(#[case] raw: &str, #[case] request_count: usize) {
    let absent = new_resolver_fixture(settings(None, 600.0, 3600.0), 1_000.0, Vec::new());
    let missing = absent.resolver.get_public_key(None).await.unwrap_err();
    assert!(matches!(missing, KeyError::MissingUrl));
    assert_eq!(
        missing.to_string(),
        "Missing JWT Public Key URL from environment."
    );
    assert!(absent.http.requests().is_empty());
    let fixture = new_resolver_fixture(
        settings(Some(raw), 600.0, 3600.0),
        1_000.0,
        vec![
            ScriptedHttp::answer(&keys(&[key("b")])),
            ScriptedHttp::answer(&keys(&[key("a")])),
        ],
    );
    let result = fixture.resolver.get_public_key(Some("a")).await;
    if request_count == 0 {
        let error = result.unwrap_err();
        assert!(matches!(&error, KeyError::NoMatchingKey { keys, .. } if keys == "[]"));
        assert_eq!(
            error.to_string(),
            "No matching public key found. keys=[], kid=a"
        );
    } else {
        assert_eq!(result.unwrap(), key("a"));
        assert_eq!(
            fixture.http.requests(),
            vec!["https://b.example.com/keys", "https://a.example.com/keys"]
        );
    }
    assert_eq!(fixture.http.requests().len(), request_count);
}

#[tokio::test]
async fn ordered_urls_keep_duplicates() {
    // the list is tried in order, each entry trimmed
    let first = "https://a.example.com/keys";
    let second = "https://b.example.com/keys";
    let raw = " https://b.example.com/keys , , https://a.example.com/keys ";
    let ordered = new_resolver_fixture(
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
    let duplicate = new_resolver_fixture(
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
    let fixture = new_resolver_fixture(settings(Some(&raw), 600.0, 3600.0), 1_000.0, outage());
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
    let fixture = new_resolver_fixture(
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
        &error,
        KeyError::NoMatchingKey { keys, kid }
            if keys == "['https://idp.example.com/jwks']" && kid.as_deref() == Some("b")
    ));
    assert_eq!(
        error.to_string(),
        "No matching public key found. keys=['https://idp.example.com/jwks'], kid=b"
    );

    // the fetch succeeded, so the keys are cached for the next caller
    let error2 = fixture
        .resolver
        .get_public_key(Some("b"))
        .await
        .unwrap_err();
    assert!(matches!(error2, KeyError::NoMatchingKey { .. }));
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
    let fixture = new_resolver_fixture(
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
