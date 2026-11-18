#![cfg(test)]

use betterlite_openai::assistants::client::ClientKind;

use super::support::{ASSISTANTS_URL, Auth, Fixture, KEY, PAGE, PAGE_EXTRA, harness, parse, wire};

pub(super) async fn check() {
    let fixture = Fixture::new();
    let request = fixture.request();
    let (sync_api, sync_transport) = harness(200, PAGE);
    let listed = sync_api.get_assistants(&request, None, None).await.unwrap();
    assert_eq!(listed.kind, ClientKind::Sync);
    assert!(listed.reply.data.is_empty());
    assert_eq!(
        sync_transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY))]
    );
    let (async_api, async_transport) = harness(200, PAGE_EXTRA);
    let page = async_api
        .async_get_assistants(&request, None)
        .await
        .unwrap();
    assert_eq!(page.kind, ClientKind::Async);
    assert_eq!(
        serde_json::to_value(&page.reply).unwrap(),
        parse(PAGE_EXTRA)
    );
    assert_eq!(
        async_transport.requests(),
        vec![wire("GET", ASSISTANTS_URL, None, Auth::Key(KEY))]
    );
}
