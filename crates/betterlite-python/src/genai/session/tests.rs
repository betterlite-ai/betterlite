#![cfg(test)]

use super::Entry;

#[test]
fn call_types_are_the_litellm_entry_names() {
    assert_eq!(Entry::Blocking.call_type(), "generate_content");
    assert_eq!(Entry::Await.call_type(), "agenerate_content");
    assert_eq!(Entry::BlockingStream.call_type(), "generate_content_stream");
    assert_eq!(Entry::AwaitStream.call_type(), "agenerate_content_stream");
}
