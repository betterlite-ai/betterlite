#![cfg(test)]

use std::time::Duration;

use betterlite_redact::secret::Redactor;

use super::cases::{Answer, Run, STABILITY_MODEL, auth_params, call, image_list, run};
use super::fixture::Reply;
use betterlite_core::api::vector_store_http_write::service::ClientKind;

pub(super) fn edit_transport_selection(redactor: &Redactor) {
    sync_selection(redactor);
    async_selection(redactor);
}

fn sync_selection(redactor: &Redactor) {
    // the factory builds the default client, a supplied async client is no match, a supplied sync one replaces it
    let from_factory = edit_run(redactor, false, None, None);
    assert_eq!(from_factory.sync_calls, 1);
    assert!(from_factory.async_calls.is_empty());
    assert_eq!(from_factory.factory_requests.len(), 1);
    let async_ignored = edit_run(redactor, false, None, Some(ClientKind::Asynchronous));
    assert_eq!(async_ignored.sync_calls, 1);
    assert!(async_ignored.supplied_requests.is_empty());
    let sync_taken = edit_run(redactor, false, None, Some(ClientKind::Sync));
    assert_eq!(sync_taken.sync_calls, 0);
    assert_eq!(sync_taken.supplied_requests.len(), 1);
}

fn async_selection(redactor: &Redactor) {
    // the default client takes the call timeout, including None, and a sync client is rejected here
    let timeout = Some(Duration::from_millis(2500));
    let passed_timeout = edit_run(redactor, true, timeout, None);
    assert_eq!(passed_timeout.async_calls, vec![timeout]);
    assert_eq!(passed_timeout.factory_requests.len(), 1);
    let no_timeout_option = edit_run(redactor, true, None, None);
    assert_eq!(no_timeout_option.async_calls, vec![None]);
    assert_eq!(no_timeout_option.factory_requests.len(), 1);
    let async_supplied = edit_run(redactor, true, None, Some(ClientKind::Asynchronous));
    assert!(async_supplied.async_calls.is_empty());
    assert_eq!(async_supplied.supplied_requests.len(), 1);
    let sync_rejected = edit_run(redactor, true, timeout, Some(ClientKind::Sync));
    assert_eq!(sync_rejected.async_calls, vec![timeout]);
    assert!(sync_rejected.supplied_requests.is_empty());
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
