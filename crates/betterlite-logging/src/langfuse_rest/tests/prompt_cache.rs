use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_util::task::TaskTracker;

use super::support::{
    Answer, FakeClock, FakeLangfuse, capture_logs, client_for, prompt_version, text_prompt,
};
use crate::langfuse_rest::cache::{Clock as _, SystemClock};

const TTL: Duration = Duration::from_secs(60);
const PATIENCE: Duration = Duration::from_secs(5);

#[test]
fn the_system_clock_reads_the_monotonic_clock() {
    let before = Instant::now();
    let now = SystemClock.now();
    let after = Instant::now();
    assert!(now >= before);
    assert!(now <= after);
}

#[tokio::test]
async fn a_cached_prompt_is_served_without_a_second_request() {
    let first_body = text_prompt("greeting", 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: first_body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    let second = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the second prompt");
    assert_eq!(prompt_version(&first), 1);
    assert_eq!(prompt_version(&second), 1);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn a_different_version_is_a_different_cache_entry() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Respond {
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let first = client
        .get_prompt("greeting", None, Some(1))
        .await
        .expect("the first prompt");
    let second = client
        .get_prompt("greeting", None, Some(2))
        .await
        .expect("the second prompt");
    assert_eq!(prompt_version(&first), 1);
    assert_eq!(prompt_version(&second), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_different_label_is_a_different_cache_entry() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Respond {
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let first = client
        .get_prompt("greeting", Some("production"), None)
        .await
        .expect("the first prompt");
    let second = client
        .get_prompt("greeting", Some("staging"), None)
        .await
        .expect("the second prompt");
    assert_eq!(prompt_version(&first), 1);
    assert_eq!(prompt_version(&second), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_different_name_is_a_different_cache_entry() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("farewell", 1);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Respond {
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    let second = client
        .get_prompt("farewell", None, None)
        .await
        .expect("the second prompt");
    assert_eq!(prompt_version(&first), 1);
    assert_eq!(prompt_version(&second), 1);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_prompt_is_fresh_before_the_ttl_has_passed() {
    let body = text_prompt("greeting", 1);
    let server = FakeLangfuse::start(vec![Answer::Respond {
        status: 200,
        body: body.clone(),
    }])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(Duration::from_secs(59));
    let fresh = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the fresh prompt");
    assert_eq!(prompt_version(&fresh), 1);
    assert_eq!(server.request_count(), 1);
}

#[tokio::test]
async fn a_prompt_is_stale_at_the_ttl_and_refreshes() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Respond {
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(TTL);
    let stale = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the stale prompt");
    assert_eq!(prompt_version(&stale), 1);
    tasks.close();
    tasks.wait().await;
    let refreshed = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the refreshed prompt");
    assert_eq!(prompt_version(&refreshed), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_stale_prompt_is_served_at_once_and_refreshed_in_the_background() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let (release, released) = oneshot::channel();
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Hold {
            release: released,
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(TTL);
    let stale = timeout(PATIENCE, client.get_prompt("greeting", None, None))
        .await
        .expect("the stale prompt is served at once")
        .expect("the stale prompt");
    assert_eq!(prompt_version(&stale), 1);
    server.wait_for_requests(2).await;
    let _released = release.send(());
    tasks.close();
    tasks.wait().await;
    let refreshed = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the refreshed prompt");
    assert_eq!(prompt_version(&refreshed), 2);
    assert_eq!(server.request_count(), 2);
}

#[tokio::test]
async fn a_zero_ttl_refreshes_on_every_hit() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Respond {
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, Duration::ZERO, &clock, &tasks);
    let first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    assert_eq!(prompt_version(&first), 1);
    let second = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the second prompt");
    assert_eq!(prompt_version(&second), 1);
    tasks.close();
    tasks.wait().await;
    assert_eq!(server.request_count(), 2);
    let third = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the third prompt");
    assert_eq!(prompt_version(&third), 2);
}

#[tokio::test]
async fn concurrent_stale_reads_spawn_one_refresh() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let (release, released) = oneshot::channel();
    let server = FakeLangfuse::start(vec![
        Answer::Respond {
            status: 200,
            body: first_body.clone(),
        },
        Answer::Hold {
            release: released,
            status: 200,
            body: second_body.clone(),
        },
    ])
    .await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(TTL);
    let stale = timeout(PATIENCE, client.get_prompt("greeting", None, None))
        .await
        .expect("the stale prompt is served at once")
        .expect("the stale prompt");
    assert_eq!(prompt_version(&stale), 1);
    server.wait_for_requests(2).await;
    let again = timeout(PATIENCE, client.get_prompt("greeting", None, None))
        .await
        .expect("the second stale prompt is served at once")
        .expect("the second stale prompt");
    assert_eq!(prompt_version(&again), 1);
    let _released = release.send(());
    tasks.close();
    tasks.wait().await;
    assert_eq!(server.request_count(), 2);
}

fn failing_refresh_script(first_body: &str, second_body: &str) -> Vec<Answer> {
    vec![
        Answer::Respond {
            status: 200,
            body: first_body.to_owned(),
        },
        Answer::Respond {
            status: 500,
            body: r#"{"message":"boom"}"#.to_owned(),
        },
        Answer::Respond {
            status: 500,
            body: r#"{"message":"boom"}"#.to_owned(),
        },
        Answer::Respond {
            status: 200,
            body: second_body.to_owned(),
        },
    ]
}

#[tokio::test]
async fn a_failed_refresh_keeps_the_stale_prompt_in_service() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(failing_refresh_script(&first_body, &second_body)).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(TTL);
    let stale = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the stale prompt");
    assert_eq!(prompt_version(&stale), 1);
    tasks.close();
    tasks.wait().await;
    assert_eq!(server.request_count(), 3);
    let stale_again = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the stale prompt again");
    assert_eq!(prompt_version(&stale_again), 1);
}

#[tokio::test]
async fn a_failed_refresh_allows_a_later_refresh() {
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(failing_refresh_script(&first_body, &second_body)).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(TTL);
    let _stale = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the stale prompt");
    tasks.close();
    tasks.wait().await;
    let stale_again = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the stale prompt again");
    assert_eq!(prompt_version(&stale_again), 1);
    tasks.wait().await;
    let refreshed = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the refreshed prompt");
    assert_eq!(prompt_version(&refreshed), 2);
    assert_eq!(server.request_count(), 4);
}

#[tokio::test]
async fn a_failed_refresh_is_logged_at_warning_level() {
    let (captured, _guard) = capture_logs();
    let first_body = text_prompt("greeting", 1);
    let second_body = text_prompt("greeting", 2);
    let server = FakeLangfuse::start(failing_refresh_script(&first_body, &second_body)).await;
    let tasks = TaskTracker::new();
    let clock = FakeClock(Arc::new(Mutex::new(Instant::now())));
    let client = client_for(&server, TTL, &clock, &tasks);
    let _first = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the first prompt");
    clock.advance(TTL);
    let _stale = client
        .get_prompt("greeting", None, None)
        .await
        .expect("the stale prompt");
    tasks.close();
    tasks.wait().await;
    let logs = captured.text();
    assert!(logs.contains("WARN"), "{logs}");
    assert!(
        logs.contains(
            "Langfuse prompt 'greeting' refresh failed, serving the cached version: status_code: 500"
        ),
        "{logs}"
    );
}
