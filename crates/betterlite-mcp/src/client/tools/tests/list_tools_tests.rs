#![cfg(test)]

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::Level;

use super::tests::fixture::{
    BrokenResource, CallOutcome, Frame, Script, TestError, build_client, capture, failing, page,
    protocol_failure, tool,
};
use crate::client::sessions::error::SessionError;
use crate::client::tools::listing::list_tools;

fn listing<Source>(pages: Vec<Frame>) -> Script<Source> {
    Script {
        pages,
        list_error: None,
        call: CallOutcome::Echo,
        progress: Vec::new(),
        hang_on_exit: false,
    }
}

#[tokio::test]
async fn lists_every_page_of_the_catalog() {
    let script = listing::<TestError>(vec![
        page(&["first"], Some("cursor-1")),
        page(&["second"], None),
    ]);
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let tools = list_tools(&mut client, Duration::from_secs(30), false)
        .await
        .unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["first", "second"]);
    assert_eq!(logs.matching("MCP client listed 2 tools").len(), 1);
    assert_eq!(logs.matching("MCP client listing tools from").len(), 1);
}

#[tokio::test]
async fn returns_an_empty_list_when_the_server_fails() {
    let script = failing(TestError::Boom("listing failed"));
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let tools = list_tools(&mut client, Duration::from_secs(30), false)
        .await
        .unwrap();
    assert!(tools.is_empty());
    let failures = logs.matching("MCP client list_tools failed");
    assert_eq!(failures.len(), 1);
    assert!(!failures[0].contains("listing failed"));
    assert_eq!(logs.count(Level::ERROR), 1);
}

#[tokio::test]
async fn re_raises_when_the_caller_owns_the_error() {
    let script = failing(TestError::Boom("listing failed"));
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let outcome = list_tools(&mut client, Duration::from_secs(30), true).await;
    match outcome {
        Err(SessionError::Runtime(error)) => assert_eq!(*error, TestError::Boom("listing failed")),
        other => panic!("expected the upstream error, got {other:?}"),
    }
    assert_eq!(logs.count(Level::ERROR), 0);
    assert_eq!(
        logs.events()
            .into_iter()
            .filter(|(level, message)| *level == Level::DEBUG
                && message.contains("MCP client list_tools failed"))
            .count(),
        1
    );
}

#[tokio::test(start_paused = true)]
async fn returns_the_partial_catalog_when_the_deadline_expires() {
    let script = listing::<TestError>(vec![
        page(&["first"], Some("cursor-1")),
        Frame {
            delay: Duration::from_secs(5),
            tools: Vec::new(),
            next_cursor: None,
        },
    ]);
    let mut client = build_client(script, Duration::from_secs(1), CancellationToken::new());
    let (logs, _guard) = capture();
    let tools = list_tools(&mut client, Duration::from_secs(1), false)
        .await
        .unwrap();
    let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["first"]);
    assert_eq!(logs.matching("listing deadline").len(), 1);
}

#[tokio::test(start_paused = true)]
async fn extends_the_deadline_to_the_longer_client_timeout() {
    let slow = Frame {
        delay: Duration::from_millis(300),
        tools: vec![tool("slow")],
        next_cursor: None,
    };
    let script = listing::<TestError>(vec![slow]);
    let mut client = build_client(script, Duration::from_secs(1), CancellationToken::new());
    let tools = list_tools(&mut client, Duration::from_millis(100), false)
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn extends_the_deadline_to_the_longer_listing_timeout() {
    let slow = Frame {
        delay: Duration::from_millis(300),
        tools: vec![tool("slow")],
        next_cursor: None,
    };
    let script = listing::<TestError>(vec![slow]);
    let mut client = build_client(script, Duration::from_millis(100), CancellationToken::new());
    let tools = list_tools(&mut client, Duration::from_secs(1), false)
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);
}

#[tokio::test]
async fn logs_a_broken_connection_diagnostic() {
    let script = failing(BrokenResource);
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    assert!(
        list_tools(&mut client, Duration::from_secs(30), false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(logs.count(Level::ERROR), 2);
    assert_eq!(
        logs.matching("detected broken connection/stream during list_tools")
            .len(),
        1
    );
}

#[tokio::test]
async fn keeps_the_broken_connection_diagnostic_quiet_when_the_caller_owns_the_error() {
    let script = failing(BrokenResource);
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    let outcome = list_tools(&mut client, Duration::from_secs(30), true).await;
    assert!(matches!(outcome, Err(SessionError::Runtime(_))));
    assert_eq!(logs.count(Level::ERROR), 0);
    assert_eq!(
        logs.matching("detected broken connection/stream during list_tools")
            .len(),
        1
    );
}

#[tokio::test]
async fn checks_only_the_error_name_for_a_broken_connection() {
    let script = Script::<TestError> {
        pages: Vec::new(),
        list_error: Some(protocol_failure()),
        call: CallOutcome::Echo,
        progress: Vec::new(),
        hang_on_exit: false,
    };
    let mut client = build_client(script, Duration::from_secs(60), CancellationToken::new());
    let (logs, _guard) = capture();
    assert!(
        list_tools(&mut client, Duration::from_secs(30), false)
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(logs.matching("MCP client list_tools failed").len(), 1);
    assert!(logs.matching("detected broken connection").is_empty());
}
