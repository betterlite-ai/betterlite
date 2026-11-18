use std::sync::Arc;

use crate::agentcore_stream::transport::HandlerKind;

use super::super::support::{BASE, Fixture, Recorded, call, data, messages, run};

const MODEL: &str = "amazon.titan-text-express-v1";
const SERIALIZED: &[u8] = br#"{"inputText": "hi", "max_tokens": 5}"#;

pub(super) fn async_sync_signed_bytes() {
    async_sends_serialized_data();
    sync_forwards_the_signed_body();
    sync_forwards_empty_signed_bytes();
    sync_without_signed_sends_data();
    sync_client_selection();
    async_client_selection();
}

fn async_sends_serialized_data() {
    let posted = posted(Some(b"c2lnbmVk"), Some(HandlerKind::Async), false);
    assert_eq!(posted.supplied.len(), 1);
    assert_eq!(posted.supplied[0].body, SERIALIZED.to_vec());
    assert!(posted.default.is_empty());
}

fn sync_forwards_the_signed_body() {
    let posted = posted(Some(b"c2lnbmVk"), Some(HandlerKind::Sync), true);
    assert_eq!(posted.supplied.len(), 1);
    assert_eq!(posted.supplied[0].body, b"c2lnbmVk".to_vec());
}

fn sync_forwards_empty_signed_bytes() {
    let posted = posted(Some(b""), Some(HandlerKind::Sync), true);
    assert_eq!(posted.supplied.len(), 1);
    assert_eq!(posted.supplied[0].body, Vec::<u8>::new());
}

fn sync_without_signed_sends_data() {
    let posted = posted(None, Some(HandlerKind::Sync), true);
    assert_eq!(posted.supplied[0].body, SERIALIZED.to_vec());
}

// _get_httpx_client(params={}) replaces a missing or async client, so the default handler takes the post.
fn sync_client_selection() {
    let mismatched = posted(None, Some(HandlerKind::Async), true);
    assert!(mismatched.supplied.is_empty());
    assert_eq!(mismatched.default.len(), 1);
    assert_eq!(mismatched.default[0].body, SERIALIZED.to_vec());

    let absent = posted(None, None, true);
    assert!(absent.supplied.is_empty());
    assert_eq!(absent.default.len(), 1);
}

fn async_client_selection() {
    let supplied = posted(None, Some(HandlerKind::Async), false);
    assert_eq!(supplied.supplied.len(), 1);
    assert!(supplied.default.is_empty());

    let absent = posted(None, None, false);
    assert!(absent.supplied.is_empty());
    assert_eq!(absent.default.len(), 1);
}

struct Posted {
    supplied: Vec<Recorded>,
    default: Vec<Recorded>,
}

fn posted(signed_json_body: Option<&[u8]>, supplied: Option<HandlerKind>, sync: bool) -> Posted {
    let messages = messages();
    let data = data();
    let mut call = call(MODEL, BASE, &data, &messages);
    call.signed_json_body = signed_json_body;
    let supplied_fixture = supplied.map(|kind| Fixture::new(kind, &[], Vec::new()));
    let default_fixture = Fixture::new(default_kind(sync), &[], Vec::new());
    let _opened = run(
        &call,
        supplied_fixture.clone(),
        Arc::clone(&default_fixture),
        sync,
    );
    let supplied_posts = match supplied_fixture {
        Some(fixture) => fixture.recorded(),
        None => Vec::new(),
    };
    Posted {
        supplied: supplied_posts,
        default: default_fixture.recorded(),
    }
}

const fn default_kind(sync: bool) -> HandlerKind {
    if sync {
        HandlerKind::Sync
    } else {
        HandlerKind::Async
    }
}
