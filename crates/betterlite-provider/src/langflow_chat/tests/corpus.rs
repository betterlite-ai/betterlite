use std::fs;

use crate::langflow_chat::reply::decode::decode_content;

#[test]
fn replays_the_langflow_chat_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/langflow_chat_reply")
        .expect("the corpus folder exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).expect("the corpus file reads");
            let _content = decode_content(&bytes, 200);
        })
        .count();
    assert!(seen > 0_usize);
}
