use std::fs;
use std::path::PathBuf;
use std::str;

use rstest::rstest;
use tokio::runtime::Builder;

use crate::image::settings::Settings as ImageSettings;
use crate::media::count::count_token_media;
use crate::media::settings::{ImageCounting, Settings};
use crate::media::wire::TokenMedia;

use super::support;

fn exact() -> Settings {
    Settings {
        image: ImageSettings::default(),
        fallback_token_count: None,
        image_counting: ImageCounting::Exact,
    }
}

fn default_count() -> Settings {
    Settings {
        image: ImageSettings::default(),
        fallback_token_count: None,
        image_counting: ImageCounting::DefaultCount,
    }
}

#[rstest]
fn replays_the_token_media_corpus(#[files("fuzz/corpus/token_media/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(media) = TokenMedia::try_from(bytes.as_slice()) else {
        return;
    };
    let fixture = support::cl100k_fixture();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let _exact = runtime.block_on(count_token_media(&fixture.texter(), &media, &exact()));
    let _default = runtime.block_on(count_token_media(
        &fixture.texter(),
        &media,
        &default_count(),
    ));
}

#[rstest]
fn replays_the_upstream_media_corpus(
    #[files("../betterlite-prompts/fuzz/corpus/media_data/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Ok(text) = str::from_utf8(&bytes) else {
        return;
    };
    let quoted = serde_json::to_string(text).unwrap();
    let input = format!(
        r#"{{"content":[{{"type":"image_url","image_url":{{"url":{quoted},"detail":"high"}}}}]}}"#
    );
    let Ok(media) = TokenMedia::try_from(input.as_bytes()) else {
        return;
    };
    let fixture = support::cl100k_fixture();
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let _counted = runtime.block_on(count_token_media(&fixture.texter(), &media, &exact()));
}
