use crate::anthropic_invokes::file_blocks::RemoteFileBlocks;
use crate::anthropic_results::tool_result::FileBlocks as _;

use super::{MapSource, maps, object};

#[test]
fn anthropic_invokes_behavior_tests_behavior_a_remote_file_block_becomes_a_document_block() {
    let data_urls = maps(&[]);
    let fetcher = MapSource {
        data_urls: &data_urls,
    };
    let blocks = RemoteFileBlocks { fetcher: &fetcher };
    let element =
        object(r#"{"type":"file","file":{"file_data":"data:application/pdf;base64,PDF"}}"#);
    assert_eq!(
        blocks.file_block(&element).unwrap(),
        object(
            r#"{"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"PDF"}}"#,
        )
    );
}
