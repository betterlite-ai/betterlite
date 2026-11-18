#![cfg(test)]

use std::path::Path;
use std::time::Duration;

use redis::aio::MultiplexedConnection;
use redis::{Client, cmd};

use crate::error::RedisFixtureError;
use crate::server::{POLL_INTERVAL, REDIS_SERVER, RedisFixture, STARTUP_LIMIT, StartOptions};

// A never-answering server only has to outlive the fixture's own readiness poll.
const TEST_LIMIT: Duration = Duration::from_millis(200);

async fn connect(url: &str) -> MultiplexedConnection {
    Client::open(url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_fixture_server_answers_ping_over_its_unix_socket() {
    let fixture = RedisFixture::start().await.unwrap();
    assert!(fixture.as_ref().starts_with("redis+unix:///"));
    let mut connection = connect(fixture.as_ref()).await;
    let reply: String = cmd("PING").query_async(&mut connection).await.unwrap();
    assert_eq!(reply, "PONG");
}

#[tokio::test]
async fn two_fixtures_keep_their_data_apart() {
    let first = RedisFixture::start().await.unwrap();
    let second = RedisFixture::start().await.unwrap();
    assert_ne!(first.as_ref(), second.as_ref());
    let mut first_connection = connect(first.as_ref()).await;
    let mut second_connection = connect(second.as_ref()).await;
    cmd("SET")
        .arg("fixture-key")
        .arg("first")
        .query_async::<()>(&mut first_connection)
        .await
        .unwrap();
    let unseen: Option<String> = cmd("GET")
        .arg("fixture-key")
        .query_async(&mut second_connection)
        .await
        .unwrap();
    assert_eq!(unseen, None);
    let seen: Option<String> = cmd("GET")
        .arg("fixture-key")
        .query_async(&mut first_connection)
        .await
        .unwrap();
    assert_eq!(seen.as_deref(), Some("first"));
}

#[tokio::test]
async fn dropping_a_fixture_stops_the_server_and_removes_its_folder() {
    let fixture = RedisFixture::start().await.unwrap();
    let process = fixture.child.id();
    let socket = Path::new(fixture.as_ref().trim_start_matches("redis+unix://")).to_path_buf();
    let folder = socket.parent().unwrap().to_path_buf();
    assert!(socket.exists());
    drop(fixture);
    assert!(!folder.exists());
    assert!(!process_is_alive(process));
}

#[tokio::test]
async fn a_missing_redis_server_binary_is_a_typed_error() {
    let options = StartOptions {
        binary: "/nonexistent/betterlite/redis-server".to_owned(),
        startup_limit: TEST_LIMIT,
        extra_args: Vec::new(),
    };
    let error = RedisFixture::launch(&options).await.unwrap_err();
    let message = error.to_string();
    assert!(matches!(&error, RedisFixtureError::MissingServer { .. }));
    assert!(message.contains("redis-server"));
    assert!(message.contains("Debian"));
}

#[tokio::test]
async fn a_server_that_never_answers_ping_fails_with_its_log() {
    let options = StartOptions {
        binary: REDIS_SERVER.to_owned(),
        startup_limit: TEST_LIMIT,
        extra_args: vec!["--requirepass".to_owned(), "not-the-password".to_owned()],
    };
    let error = RedisFixture::launch(&options).await.unwrap_err();
    match error {
        RedisFixtureError::NotReady { log, .. } => assert!(log.contains("Redis version")),
        other @ RedisFixtureError::MissingServer { .. }
        | other @ RedisFixtureError::Spawn { .. }
        | other @ RedisFixtureError::Folder { .. }
        | other @ RedisFixtureError::Log { .. }
        | other @ RedisFixtureError::NonUtf8Path { .. }
        | other @ RedisFixtureError::BlockingTaskAborted
        | other @ RedisFixtureError::BlockingTaskFailed { .. } => {
            panic!("expected a readiness failure, got {other:?}")
        }
    }
}

#[test]
fn the_startup_budget_and_poll_interval_match_the_design() {
    assert_eq!(STARTUP_LIMIT, Duration::from_secs(5));
    assert_eq!(POLL_INTERVAL, Duration::from_millis(2));
}

#[cfg(unix)]
#[test]
fn a_socket_path_that_is_not_utf8_is_a_typed_error() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;
    use std::path::PathBuf;

    use crate::server::socket_url;

    let socket = PathBuf::from(OsString::from_vec(b"/tmp/redis-\xff/redis.sock".to_vec()));
    let error = socket_url(&socket).unwrap_err();
    assert!(matches!(error, RedisFixtureError::NonUtf8Path { .. }));
}

#[cfg(target_os = "linux")]
fn process_is_alive(process: u32) -> bool {
    Path::new("/proc").join(process.to_string()).exists()
}

#[cfg(not(target_os = "linux"))]
fn process_is_alive(_process: u32) -> bool {
    false
}
