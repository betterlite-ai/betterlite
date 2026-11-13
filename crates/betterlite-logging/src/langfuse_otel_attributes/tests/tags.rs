#![cfg(test)]

use super::support::{attr, metadata_attributes};

#[test]
fn tags_serialize_the_list_to_json_text() {
    assert_eq!(
        metadata_attributes(r#"{"tags":["alpha","beta"]}"#),
        vec![attr("langfuse.trace.tags", r#"["alpha","beta"]"#)]
    );
}
