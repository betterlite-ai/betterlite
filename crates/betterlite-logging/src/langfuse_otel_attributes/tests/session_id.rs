#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn session_id_writes_the_session_id() {
    assert_eq!(
        metadata_attributes(r#"{"session_id":"session-1"}"#),
        vec![attr("session.id", "session-1")]
    );
}
