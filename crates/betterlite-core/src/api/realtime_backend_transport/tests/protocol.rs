use crate::api::realtime_backend_transport::protocol::{ClientSlot, client_wants_beta};
use crate::api::realtime_contracts::transport::{
    ClientWebSocket as _, ScopedWebSocket as _, WebSocketScope,
};

use super::harness::{HarnessOptions, client_handle};

#[test]
fn client_wants_beta_reads_the_scope_header() {
    assert!(!client_wants_beta(&WebSocketScope::default()));
    let beta = WebSocketScope {
        headers: vec![("OpenAI-Beta".to_owned(), "realtime=v1".to_owned())],
    };
    assert!(client_wants_beta(&beta));
}

#[test]
fn client_slot_keeps_the_handle_scope() {
    let mut handle = client_handle(&HarnessOptions::default());
    handle.scope = WebSocketScope {
        headers: vec![("OpenAI-Beta".to_owned(), "realtime=v1".to_owned())],
    };
    let slot = ClientSlot { handle };
    assert_eq!(slot.scope().headers.len(), 1);
}

#[tokio::test]
async fn client_slot_forwards_receive_text() {
    let mut slot = ClientSlot {
        handle: client_handle(&HarnessOptions::default()),
    };
    let error = slot.receive_text().await.unwrap_err();
    assert_eq!(error.message, "the fake client holds no frames");
}
