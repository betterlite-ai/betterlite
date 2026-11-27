#![cfg(test)]

use std::time::Duration;

use betterlite_redact::secret::Redactor;

use super::cases::{Answer, Run, STABILITY_MODEL, auth_params, call, image_list, run};
use super::fixture::Reply;
use crate::image_call::transport::ClientKind;

pub(super) fn edit_transport_selection(redactor: &Redactor) {
    // sync: the factory builds the default client, a supplied async client is no match, a supplied sync one replaces it
    let from_factory = edit_run(redactor, false, None, None);
    assert_eq!(from_factory.sync_calls, 1);
    assert!(from_factory.async_calls.is_empty());
    assert_eq!(from_factory.factory_requests.len(), 1);
    let given_async = edit_run(redactor, false, None, Some(ClientKind::Async));
    assert_eq!(given_async.sync_calls, 1);
    assert!(given_async.supplied_requests.is_empty());
    let given_sync = edit_run(redactor, false, None, Some(ClientKind::Sync));
    assert_eq!(given_sync.sync_calls, 0);
    assert_eq!(given_sync.supplied_requests.len(), 1);
    // async: the default client takes the call timeout, including None, and a sync client is rejected here
    let timeout = Some(Duration::from_millis(2500));
    let timed = edit_run(redactor, true, timeout, None);
    assert_eq!(timed.async_calls, vec![timeout]);
    assert_eq!(timed.factory_requests.len(), 1);
    let untimed = edit_run(redactor, true, None, None);
    assert_eq!(untimed.async_calls, vec![None]);
    assert_eq!(untimed.factory_requests.len(), 1);
    let supplied_async = edit_run(redactor, true, None, Some(ClientKind::Async));
    assert!(supplied_async.async_calls.is_empty());
    assert_eq!(supplied_async.supplied_requests.len(), 1);
    let rejected_sync = edit_run(redactor, true, timeout, Some(ClientKind::Sync));
    assert_eq!(rejected_sync.async_calls, vec![timeout]);
    assert!(rejected_sync.supplied_requests.is_empty());
}

fn edit_run(
    redactor: &Redactor,
    aimage_edit: bool,
    timeout: Option<Duration>,
    supplied: Option<ClientKind>,
) -> Run {
    let images = image_list(&["aW1n"]);
    let mut edit_call = call(STABILITY_MODEL, Some("edit it"), auth_params());
    edit_call.images = &images;
    edit_call.aimage_edit = aimage_edit;
    edit_call.timeout = timeout;
    let answer = Answer {
        reply: Reply::ok(r#"{"images": ["aQ=="]}"#),
        supplied,
    };
    run(edit_call, "{}", answer, redactor)
}
