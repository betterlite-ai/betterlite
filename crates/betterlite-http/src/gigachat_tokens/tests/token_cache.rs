use secrecy::SecretString;

use crate::gigachat_tokens::authenticator::GigachatAuthenticator;
use crate::gigachat_tokens::cache::TokenCache;
use crate::gigachat_tokens::request::GigachatTokenRequest;
use crate::gigachat_tokens::tests::support;
use crate::gigachat_tokens::tokens::GigachatTokens as _;

const CREDENTIALS: &str = "Y2xpZW50OnNlY3JldA==";

fn request(auth_url: &str) -> GigachatTokenRequest {
    GigachatTokenRequest {
        access_token: None,
        credentials: None,
        scope: None,
        auth_url: Some(auth_url.to_owned()),
    }
}

fn authenticator(auth_url: &str) -> GigachatAuthenticator<support::ScriptedRuntime> {
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        ("GIGACHAT_AUTH_URL", auth_url),
    ]);
    GigachatAuthenticator::new(
        settings,
        support::client(),
        support::ScriptedRuntime::new(&[0]),
    )
}

#[tokio::test]
async fn token_cache_keys_on_the_first_sixteen_characters() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":4102444800000}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":4102444800000}");
    let auth = authenticator(&server.url);
    let mut first = request(&server.url);
    first.credentials = Some(SecretString::from("abcdefghijklmnop-one"));
    let mut second = request(&server.url);
    second.credentials = Some(SecretString::from("abcdefghijklmnop-two"));
    let mut third = request(&server.url);
    third.credentials = Some(SecretString::from("abcdefghijklmnoq-three"));

    let first_token = auth
        .access_token(&first)
        .await
        .expect("the first credential fetches a token");
    let second_token = auth
        .access_token(&second)
        .await
        .expect("the shared key serves the first token");
    let third_token = auth
        .access_token(&third)
        .await
        .expect("a different prefix fetches a second token");

    assert_eq!(support::text(&first_token), "token-1");
    assert_eq!(support::text(&second_token), "token-1");
    assert_eq!(support::text(&third_token), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_serves_the_first_token_for_a_new_scope() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":4102444800000}");
    let auth = authenticator(&server.url);
    let mut first = request(&server.url);
    first.scope = Some("GIGACHAT_API_PERS".to_owned());
    let mut second = request(&server.url);
    second.scope = Some("GIGACHAT_API_CORP".to_owned());

    let first_token = auth
        .access_token(&first)
        .await
        .expect("the first scope fetches a token");
    let second_token = auth
        .access_token(&second)
        .await
        .expect("the same credentials share the cached token");

    assert_eq!(support::text(&first_token), "token-1");
    assert_eq!(support::text(&second_token), "token-1");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn token_cache_counts_characters_not_bytes() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":4102444800000}");
    let auth = authenticator(&server.url);
    let prefix = "\u{e9}".repeat(16);
    let mut first = request(&server.url);
    first.credentials = Some(SecretString::from(format!("{prefix}-one")));
    let mut second = request(&server.url);
    second.credentials = Some(SecretString::from(format!("{prefix}-two")));

    let first_token = auth
        .access_token(&first)
        .await
        .expect("the multi-byte credential fetches a token");
    let second_token = auth
        .access_token(&second)
        .await
        .expect("the same sixteen characters share the cache entry");

    assert_eq!(support::text(&first_token), "token-1");
    assert_eq!(support::text(&second_token), "token-1");
    assert_eq!(server.requests().len(), 1);
}

#[tokio::test]
async fn token_cache_serves_a_token_until_sixty_seconds_before_its_expiry() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":60100}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":60200}");
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(
        settings,
        support::client(),
        support::ScriptedRuntime::new(&[0, 0, 99, 100]),
    );
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("the cached token is served one millisecond before the buffer");
    let third = auth
        .access_token(&supplied)
        .await
        .expect("the cache expires exactly at the buffer");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-1");
    assert_eq!(support::text(&third), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_stores_nothing_when_the_remaining_ttl_is_zero() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":60000}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":60000}");
    let auth = authenticator(&server.url);
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("a zero remaining ttl is not cached");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_stores_nothing_at_the_exact_expiry_boundary() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":61000}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":61000}");
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    // the clock steps back one millisecond, which exposes an entry stored at the boundary
    let auth = GigachatAuthenticator::new(
        settings,
        support::client(),
        support::ScriptedRuntime::new(&[1_000, 1_000, 999, 999]),
    );
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("a boundary expiry is not stored");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_stores_nothing_when_the_expiry_is_missing() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\"}");
    server.answer(200, "{\"tok\":\"token-2\"}");
    let auth = authenticator(&server.url);
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("a missing expiry is not cached");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_stores_nothing_for_a_zero_expiry() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":0}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":0}");
    let auth = authenticator(&server.url);
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("a zero expiry is not cached");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_stores_nothing_for_a_zero_string_expiry() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":\"0\"}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":\"0\"}");
    let auth = authenticator(&server.url);
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("a parsed zero expiry is not cached");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_reads_the_clock_again_after_the_reply() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":61000}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":61000}");
    let settings = support::settings(&[
        ("GIGACHAT_CREDENTIALS", CREDENTIALS),
        ("GIGACHAT_AUTH_URL", server.url.as_str()),
    ]);
    let auth = GigachatAuthenticator::new(
        settings,
        support::client(),
        support::ScriptedRuntime::new(&[0, 1_000]),
    );
    let supplied = request(&server.url);

    let first = auth
        .access_token(&supplied)
        .await
        .expect("the first call fetches");
    let second = auth
        .access_token(&supplied)
        .await
        .expect("the post-reply clock decides the ttl");

    assert_eq!(support::text(&first), "token-1");
    assert_eq!(support::text(&second), "token-2");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_holds_at_most_two_hundred_entries() {
    let cache = TokenCache::default();
    support::insert_entries(&cache, 200, 1_000_000, 0).await;
    cache
        .insert(
            "key-200".to_owned(),
            SecretString::from("token-200"),
            1_000_200,
            0,
        )
        .await;

    assert!(cache.lookup("key-0", 0).await.is_none());
    assert_eq!(
        cache
            .lookup("key-1", 0)
            .await
            .map(|token| support::text(&token)),
        Some("token-1".to_owned())
    );
    assert!(cache.lookup("key-199", 0).await.is_some());
    assert!(cache.lookup("key-200", 0).await.is_some());
}

#[tokio::test]
async fn token_cache_keeps_the_old_expiry_when_a_live_key_is_replaced() {
    let cache = TokenCache::default();
    cache
        .insert("key".to_owned(), SecretString::from("first"), 120_000, 0)
        .await;
    cache
        .insert("key".to_owned(), SecretString::from("second"), 1_000_000, 0)
        .await;

    let replaced = cache
        .lookup("key", 59_999)
        .await
        .expect("the replaced token is served until the old expiry");
    assert_eq!(support::text(&replaced), "second");
    assert!(cache.lookup("key", 60_000).await.is_none());
}

#[tokio::test]
async fn token_cache_replaces_an_expired_entry_with_its_new_expiry() {
    let cache = TokenCache::default();
    cache
        .insert("key".to_owned(), SecretString::from("first"), 120_000, 0)
        .await;
    cache
        .insert(
            "key".to_owned(),
            SecretString::from("second"),
            180_000,
            60_000,
        )
        .await;

    let replaced = cache
        .lookup("key", 60_000)
        .await
        .expect("an expired entry is dropped before the insert");

    assert_eq!(support::text(&replaced), "second");
}

#[tokio::test]
async fn token_cache_concurrent_misses_each_fetch_their_own_token() {
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":4102444800000}");
    server.answer(200, "{\"tok\":\"token-2\",\"exp\":4102444800000}");
    let auth = authenticator(&server.url);
    let supplied = request(&server.url);

    let (first, second) = tokio::join!(auth.access_token(&supplied), auth.access_token(&supplied));

    first.expect("the first concurrent call fetches a token");
    second.expect("the second concurrent call fetches a token");
    assert_eq!(server.requests().len(), 2);
}

#[tokio::test]
async fn token_cache_hit_logs_the_cached_token() {
    let (logs, _guard) = support::Logs::capture();
    let server = support::Server::start().await;
    server.answer(200, "{\"tok\":\"token-1\",\"exp\":4102444800000}");
    let auth = authenticator(&server.url);
    let supplied = request(&server.url);

    auth.access_token(&supplied)
        .await
        .expect("the first call fetches");
    auth.access_token(&supplied)
        .await
        .expect("the second call is served from the cache");

    assert!(logs.text().contains("Using cached GigaChat access token"));
}
