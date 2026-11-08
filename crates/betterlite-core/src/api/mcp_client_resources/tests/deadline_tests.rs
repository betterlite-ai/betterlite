#![cfg(test)]

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use betterlite_config::sections::mcp_tool_helpers::Settings as McpToolHelperSettings;
use betterlite_mcp::client::sessions::error::SessionError;
use serde_json::Map;

use super::fixture::{
    FakeRuntime, Script, ScriptedFailure, Step, capture, protocol, resolve_session_step,
    session_client,
};
use crate::api::mcp_client_resources::operations::McpResourceOperations as _;
use crate::api::mcp_client_resources::pagination::{PageFailure, list_optional_pages};
use crate::api::mcp_client_resources::settings::{DeadlineError, ResourceSettings};
use crate::api::mcp_client_resources::types::{Capability, Prompt, PromptPage, ServerCapabilities};

fn page(names: &[&str], next: Option<&str>) -> PromptPage {
    PromptPage {
        prompts: names
            .iter()
            .map(|name| Prompt {
                name: (*name).to_owned(),
                description: None,
                arguments: None,
                extra: Map::new(),
            })
            .collect(),
        next_cursor: next.map(str::to_owned),
    }
}

fn settings(listing_seconds: u64) -> ResourceSettings {
    ResourceSettings {
        listing_timeout: Duration::from_secs(listing_seconds),
    }
}

fn prompt_names(items: &[Prompt]) -> Vec<&str> {
    items.iter().map(|prompt| prompt.name.as_str()).collect()
}

fn prompts_script(steps: Vec<Step<PromptPage>>) -> Script {
    let mut script = Script::with_capabilities(None);
    script.capabilities = Some(ServerCapabilities {
        prompts: Some(Capability::default()),
        resources: None,
    });
    script.prompts = VecDeque::from(steps);
    script
}

#[tokio::test]
async fn a_repeated_cursor_returns_the_pages_collected_so_far() {
    let (captured, _guard) = capture();
    let mut pages = VecDeque::from([
        page(&["one"], Some("same")),
        page(&["two"], Some("same")),
        page(&["three"], None),
    ]);
    let fetch = async move |_cursor: Option<String>| {
        Ok::<_, SessionError<ScriptedFailure>>(pages.pop_front().expect("page"))
    };
    let items = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::from_secs(30),
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await
    .unwrap();
    assert_eq!(prompt_names(&items), ["one", "two"]);
    assert!(captured.text().contains("repeated a cursor"));
}

#[tokio::test]
async fn an_empty_cursor_ends_the_walk() {
    let fetches = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&fetches);
    let mut pages = VecDeque::from([page(&["one"], Some("")), page(&["two"], None)]);
    let fetch = async move |cursor: Option<String>| {
        assert!(cursor.is_none());
        count.fetch_add(1, Ordering::Relaxed);
        Ok::<_, SessionError<ScriptedFailure>>(pages.pop_front().expect("page"))
    };
    let items = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::from_secs(30),
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await
    .unwrap();
    assert_eq!(prompt_names(&items), ["one"]);
    assert_eq!(fetches.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn the_page_limit_returns_every_page_it_collected() {
    let (captured, _guard) = capture();
    let mut count = 0_usize;
    let fetch = async move |_cursor: Option<String>| {
        count = count.saturating_add(1);
        Ok::<_, SessionError<ScriptedFailure>>(PromptPage {
            prompts: vec![Prompt {
                name: format!("item-{count}"),
                description: None,
                arguments: None,
                extra: Map::new(),
            }],
            next_cursor: Some(format!("cursor-{count}")),
        })
    };
    let items = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::from_secs(60),
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await
    .unwrap();
    assert_eq!(items.len(), 1_000);
    assert!(captured.text().contains("page limit"));
}

#[tokio::test]
async fn an_expired_deadline_returns_without_fetching() {
    let (captured, _guard) = capture();
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let fetch = async move |_cursor: Option<String>| {
        count.fetch_add(1, Ordering::Relaxed);
        Ok::<_, SessionError<ScriptedFailure>>(page(&["one"], None))
    };
    let items = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::ZERO,
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await
    .unwrap();
    assert!(items.is_empty());
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(captured.text().contains("deadline"));
}

#[tokio::test(start_paused = true)]
async fn a_page_past_the_deadline_returns_what_arrived() {
    let (captured, _guard) = capture();
    let mut pages = VecDeque::from([
        Step::Delayed {
            value: page(&["one"], Some("second")),
            delay: Duration::from_secs(10),
        },
        Step::Delayed {
            value: page(&["two"], None),
            delay: Duration::from_secs(100),
        },
    ]);
    let fetch = async move |_cursor: Option<String>| {
        resolve_session_step::<ScriptedFailure, _>(pages.pop_front().expect("page")).await
    };
    let items = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::from_secs(60),
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await
    .unwrap();
    assert_eq!(prompt_names(&items), ["one"]);
    assert!(captured.text().contains("deadline"));
}

#[tokio::test(start_paused = true)]
async fn a_later_page_method_not_found_raises_after_the_first_page() {
    let mut count = 0_usize;
    let fetch = async move |_cursor: Option<String>| {
        count = count.saturating_add(1);
        if count == 1 {
            Ok::<_, SessionError<ScriptedFailure>>(page(&["one"], Some("second")))
        } else {
            Err(SessionError::Protocol(protocol(-32_601, "changed")))
        }
    };
    let outcome = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::from_secs(60),
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await;
    match outcome {
        Err(PageFailure::Unavailable(failure)) => assert_eq!(failure.code, -32_601_i32),
        Ok(_) => panic!("expected the later-page failure to raise"),
        Err(PageFailure::Session(error)) => {
            panic!("expected the unavailable failure, got {error:?}")
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_later_page_past_the_whole_deadline_returns_what_arrived() {
    let mut pages = VecDeque::from([
        Step::Delayed {
            value: page(&["one"], Some("second")),
            delay: Duration::from_secs(15),
        },
        Step::Delayed {
            value: page(&["two"], None),
            delay: Duration::from_secs(10),
        },
    ]);
    let fetch = async move |_cursor: Option<String>| {
        resolve_session_step::<ScriptedFailure, _>(pages.pop_front().expect("page")).await
    };
    let items = list_optional_pages::<ScriptedFailure, PromptPage, Prompt, _, _>(
        Duration::from_secs(20),
        fetch,
        |page: &PromptPage| page.prompts.as_slice(),
    )
    .await
    .unwrap();
    assert_eq!(prompt_names(&items), ["one"]);
}

#[tokio::test(start_paused = true)]
async fn a_single_page_past_the_listing_timeout_still_succeeds() {
    let script = prompts_script(vec![Step::Delayed {
        value: page(&["slow"], None),
        delay: Duration::from_secs(40),
    }]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let items = client.list_prompts(&settings(30), true).await.unwrap();
    assert_eq!(prompt_names(&items), ["slow"]);
}

#[tokio::test(start_paused = true)]
async fn the_listing_timeout_extends_a_shorter_client_timeout() {
    let script = prompts_script(vec![Step::Delayed {
        value: page(&["slow"], None),
        delay: Duration::from_secs(20),
    }]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(15));
    let items = client.list_prompts(&settings(30), true).await.unwrap();
    assert_eq!(prompt_names(&items), ["slow"]);
}

#[test]
fn the_listing_timeout_comes_from_the_mcp_tool_helper_settings() {
    let configured = McpToolHelperSettings {
        client_timeout: 60.0_f64,
        tool_listing_timeout: 12.0_f64,
    };
    assert_eq!(
        ResourceSettings::from_config(&configured)
            .unwrap()
            .listing_timeout,
        Duration::from_secs(12)
    );
}

#[test]
fn an_invalid_listing_timeout_is_a_deadline_error() {
    let settings = McpToolHelperSettings {
        client_timeout: 60.0_f64,
        tool_listing_timeout: -1.0_f64,
    };
    let error = ResourceSettings::from_config(&settings).unwrap_err();
    assert!(matches!(error, DeadlineError::ListingTimeout { .. }));
}

#[tokio::test(start_paused = true)]
async fn a_page_past_the_effective_deadline_returns_partial_results() {
    let (captured, _guard) = capture();
    let script = prompts_script(vec![
        Step::Delayed {
            value: page(&["one"], Some("second")),
            delay: Duration::from_secs(10),
        },
        Step::Delayed {
            value: page(&["two"], None),
            delay: Duration::from_secs(100),
        },
    ]);
    let runtime = FakeRuntime::<ScriptedFailure>::new(script);
    let mut client = session_client(runtime, Duration::from_secs(60));
    let items = client.list_prompts(&settings(30), true).await.unwrap();
    assert_eq!(prompt_names(&items), ["one"]);
    assert!(captured.text().contains("deadline"));
}
