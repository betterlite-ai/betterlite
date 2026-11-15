use std::fs;
use std::path::{Path, PathBuf};

use crate::videos::error::VideoError;
use crate::videos::reply::{VideoReply, decode_video_reply};

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let reply = decode_video_reply(&bytes);
    if name.starts_with("valid-video") || name.starts_with("boundary-video") {
        return matches!(reply, Ok(VideoReply::Video(_)));
    }
    if name.starts_with("valid-list") || name.starts_with("boundary-list") {
        return matches!(reply, Ok(VideoReply::List(_)));
    }
    if name.starts_with("malformed") || name.starts_with("truncated") {
        return matches!(reply, Err(VideoError::Reply { .. }));
    }
    panic!("unknown corpus category {name}");
}

#[test]
fn replays_the_openai_videos_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_videos");
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!seeds.is_empty());
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
