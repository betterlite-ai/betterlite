#![cfg(test)]

mod fixtures;
mod openai_video_identifiers_cases;

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;

use crate::openai_video_identifiers::ids::{DecodedProviderId, decode_provider_video_id};

fn is_fallback(decoded: &DecodedProviderId) -> bool {
    match decoded {
        DecodedProviderId::Video(video) => video.custom_llm_provider.is_none(),
        DecodedProviderId::Character(character) => character.custom_llm_provider.is_none(),
        DecodedProviderId::Invalid => false,
    }
}

#[test]
fn replays_the_openai_video_identifiers_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_video_identifiers");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .expect("utf-8 file name");
            let decoded = decode_provider_video_id(&bytes);
            if name.starts_with("valid") || name.starts_with("boundary") {
                assert!(
                    matches!(
                        decoded,
                        DecodedProviderId::Video(_) | DecodedProviderId::Character(_)
                    ),
                    "{name}"
                );
                assert!(!is_fallback(&decoded), "{name}");
            } else {
                assert!(
                    matches!(decoded, DecodedProviderId::Invalid) || is_fallback(&decoded),
                    "{name}"
                );
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
