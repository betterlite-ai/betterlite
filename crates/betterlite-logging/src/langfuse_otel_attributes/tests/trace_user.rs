#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn trace_user_id_writes_the_user_id() {
    assert_eq!(
        metadata_attributes(r#"{"trace_user_id":"user-1"}"#),
        vec![attr("user.id", "user-1")]
    );
}
