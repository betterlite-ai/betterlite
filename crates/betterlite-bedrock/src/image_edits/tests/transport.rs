#![cfg(test)]

use std::time::Duration;

use betterlite_redact::secret::Redactor;

use super::cases::{Answer, STABILITY_MODEL, answered, auth_params, call, image_list, run};
use super::fixture::Reply;
use crate::image_call::transport::ClientKind;

pub(super) fn edit_transport_selection(redactor: &Redactor) {
    sync_default_client(redactor);
    supplied_async_on_sync(redactor);
    supplied_sync_client(redactor);
    async_default_client(redactor);
    async_none_timeout(redactor);
    supplied_async_client(redactor);
    sync_client_on_async(redactor);
}

fn sync_default_client(redactor: &Redactor) {
    // sync: the factory builds the default client, a supplied sync client replaces it
    let supplied = image_list(&["aW1n"]);
    let mut default_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    default_call.images = &supplied;
    let built = run(
        default_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: None,
        },
        redactor,
    );
    assert_eq!(built.sync_calls, 1);
    assert!(built.async_calls.is_empty());
    assert_eq!(built.factory_requests.len(), 1);
}

fn supplied_async_on_sync(redactor: &Redactor) {
    // an async client on a sync call is not a match, so the factory builds the sync one
    let supplied = image_list(&["aW1n"]);
    let mut async_given_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_given_call.images = &supplied;
    let ignored = run(
        async_given_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: Some(ClientKind::Async),
        },
        redactor,
    );
    assert_eq!(ignored.sync_calls, 1);
    assert!(ignored.supplied_requests.is_empty());
}

fn supplied_sync_client(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    let mut sync_given_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    sync_given_call.images = &supplied;
    let given = run(
        sync_given_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(given.sync_calls, 0);
    assert_eq!(given.supplied_requests.len(), 1);
}

fn async_default_client(redactor: &Redactor) {
    // async: the default client takes the call timeout
    let supplied = image_list(&["aW1n"]);
    let mut async_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_call.images = &supplied;
    async_call.aimage_edit = true;
    async_call.timeout = Some(Duration::from_millis(2500));
    let built = run(
        async_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: None,
        },
        redactor,
    );
    assert_eq!(built.async_calls, vec![Some(Duration::from_millis(2500))]);
    assert_eq!(built.factory_requests.len(), 1);
}

fn async_none_timeout(redactor: &Redactor) {
    // a caller with no timeout hands the async client a None timeout all the same
    let supplied = image_list(&["aW1n"]);
    let mut untimed_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    untimed_call.images = &supplied;
    untimed_call.aimage_edit = true;
    let built = run(
        untimed_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: None,
        },
        redactor,
    );
    assert_eq!(built.async_calls, vec![None]);
    assert_eq!(built.factory_requests.len(), 1);
}

fn supplied_async_client(redactor: &Redactor) {
    let supplied = image_list(&["aW1n"]);
    let mut async_supplied_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    async_supplied_call.images = &supplied;
    async_supplied_call.aimage_edit = true;
    let given = run(
        async_supplied_call,
        "{}",
        Answer {
            reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
            supplied: Some(ClientKind::Async),
        },
        redactor,
    );
    assert!(given.async_calls.is_empty());
    assert_eq!(given.supplied_requests.len(), 1);
}

fn sync_client_on_async(redactor: &Redactor) {
    // a sync client on the async path is not accepted, so the factory builds the async one
    let supplied = image_list(&["aW1n"]);
    let mut sync_on_async_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    sync_on_async_call.images = &supplied;
    sync_on_async_call.aimage_edit = true;
    sync_on_async_call.timeout = Some(Duration::from_millis(2500));
    let replaced = run(
        sync_on_async_call,
        "{}",
        answered(Reply::ok(r#"{"images": ["aQ=="]}"#)),
        redactor,
    );
    assert_eq!(
        replaced.async_calls,
        vec![Some(Duration::from_millis(2500))]
    );
    assert!(replaced.supplied_requests.is_empty());
}
