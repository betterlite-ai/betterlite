#![cfg(test)]

use betterlite_http::client_kind::ClientKind;
use betterlite_openai::assistants::records::AssistantPage;

use crate::azure_assistant_definitions::operations::{DefinitionCall, DefinitionReply};

use super::support::{ASSISTANTS_URL, Auth, Fixture, KEY, PAGE, PAGE_EXTRA, harness, wire};

pub(super) async fn check() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (sync_api, sync_transport) = harness(200, PAGE);
    let listed = sync_api
        .call(&request, None, None, DefinitionCall::List)
        .await
        .unwrap();
    assert_eq!(listed.kind, ClientKind::Sync);
    let DefinitionReply::Page(sync_page) = listed.reply else {
        panic!("a list call returns a page");
    };
    assert!(sync_page.data.is_empty());
    assert_eq!(
        sync_transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY))]
    );
    let (async_api, async_transport) = harness(200, PAGE_EXTRA);
    let async_listed = async_api
        .call(&request, None, Some(true), DefinitionCall::List)
        .await
        .unwrap();
    assert_eq!(async_listed.kind, ClientKind::Asynchronous);
    let DefinitionReply::Page(page) = async_listed.reply else {
        panic!("a list call returns a page");
    };
    let expected: AssistantPage = serde_json::from_str(PAGE_EXTRA).unwrap();
    assert_eq!(*page, expected);
    assert_eq!(
        async_transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY))]
    );
}
