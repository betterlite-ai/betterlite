use std::fs;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use rstest::rstest;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;
use std::fmt::Write as _;

use crate::engine::assets;
use crate::engine::fetch::TiktokenFetcher;
use crate::engine::fetch::gpt2::bpe_file;
use crate::engine::huggingface::HuggingFaceEngine;
use crate::engine::tests::support::{Reply, Server, client};
use crate::engine::tiktoken::{Encoding, TiktokenEngine};

fn tiktoken(encoding: Encoding) -> TiktokenEngine {
    let data = assets::bundled(encoding).unwrap();
    TiktokenEngine::from_bpe_file(encoding, &data).unwrap()
}

fn anthropic() -> HuggingFaceEngine {
    HuggingFaceEngine::from_bytes(&assets::joined(assets::ANTHROPIC_PARTS)).unwrap()
}

#[rstest]
#[case::cl100k_ascii(Encoding::Cl100kBase, "hello world", &[15339, 1917])]
#[case::cl100k_unicode(Encoding::Cl100kBase, "H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b}", &[39, 19010, 385, 11, 220, 3574, 244, 98220, 0, 62904, 233])]
#[case::cl100k_whitespace(Encoding::Cl100kBase, "a   b\n\tc", &[64, 256, 293, 198, 1470])]
#[case::cl100k_specials(Encoding::Cl100kBase, "<|endoftext|>", &[27, 91, 8862, 728, 428, 91, 29])]
#[case::cl100k_padding(Encoding::Cl100kBase, "  leading and trailing  ", &[220, 6522, 323, 28848, 256])]
#[case::cl100k_newlines(Encoding::Cl100kBase, "one\ntwo\n\nthree", &[606, 198, 20375, 271, 28956])]
#[case::cl100k_zwj(Encoding::Cl100kBase, "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}", &[9468, 239, 101, 378, 235, 9468, 239, 102, 378, 235, 9468, 239, 100, 378, 235, 9468, 239, 99])]
#[case::o200k_ascii(Encoding::O200kBase, "hello world", &[24912, 2375])]
#[case::o200k_unicode(Encoding::O200kBase, "H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b}", &[39, 377, 72807, 11, 185_558, 0, 61138, 233])]
#[case::o200k_whitespace(Encoding::O200kBase, "a   b\n\tc", &[64, 256, 287, 198, 2736])]
#[case::o200k_specials(Encoding::O200kBase, "<|endoftext|>", &[27, 91, 419, 1440, 919, 91, 29])]
#[case::p50k_ascii(Encoding::P50kBase, "hello world", &[31373, 995])]
#[case::p50k_whitespace(Encoding::P50kBase, "a   b\n\tc", &[64, 50257, 275, 198, 197, 66])]
#[case::p50k_edit_ascii(Encoding::P50kEdit, "hello world", &[31373, 995])]
#[case::harmony_ascii(Encoding::O200kHarmony, "hello world", &[24912, 2375])]
#[case::harmony_newlines(Encoding::O200kHarmony, "one\ntwo\n\nthree", &[690, 198, 38397, 279, 47660])]
fn tiktoken_matches_python(#[case] encoding: Encoding, #[case] text: &str, #[case] ids: &[u32]) {
    assert_eq!(tiktoken(encoding).encode(text).unwrap(), ids);
}

#[rstest]
#[case::ascii("hello world", &[9381, 2253])]
#[case::unicode("H\u{e9}llo, \u{4e16}\u{754c}! \u{1f44b}", &[44, 64032, 354, 16, 225, 1499, 249, 37413, 5, 41270, 244, 238])]
#[case::whitespace("a   b\n\tc", &[69, 261, 301, 203, 202, 71])]
#[case::specials("<|endoftext|>", &[32, 96, 441, 1392, 676, 96, 34])]
#[case::newlines("one\ntwo\n\nthree", &[396, 203, 7156, 203, 203, 11862])]
fn anthropic_matches_python(#[case] text: &str, #[case] ids: &[u32]) {
    assert_eq!(anthropic().encode(text).unwrap(), ids);
}

#[test]
fn decode_matches_python() {
    let engine = tiktoken(Encoding::Cl100kBase);
    assert_eq!(engine.decode(&[15339, 1917]).unwrap(), "hello world");
    assert_eq!(engine.decode(&[]).unwrap(), "");
    assert_eq!(engine.decode(&[100_257]).unwrap(), "<|endoftext|>");
    assert_eq!(engine.decode(&[128]).unwrap(), "\u{fffd}");
    let _failure = engine.decode(&[999_999_999]).unwrap_err();
    assert_eq!(anthropic().decode(&[9381, 2253]).unwrap(), "hello world");
    assert_eq!(anthropic().decode(&[0, 9381, 2253]).unwrap(), "hello world");
}

fn synthetic_bpe_file() -> Vec<u8> {
    let mut file = String::new();
    for byte in 0_u8..=255 {
        writeln!(file, "{} {byte}", STANDARD.encode([byte])).unwrap();
    }
    writeln!(file, "{} 256", STANDARD.encode(b"he")).unwrap();
    writeln!(file, "{} 257", STANDARD.encode(b"ll")).unwrap();
    file.into_bytes()
}

async fn r50k_fixture() -> (Server, tempfile::TempDir, TiktokenFetcher, String) {
    let body = synthetic_bpe_file();
    let sha256 = hex::encode(Sha256::digest(&body));
    let server = Server::start(move |_request| Reply::Response {
        status: 200,
        headers: Vec::new(),
        body: body.clone(),
    })
    .await;
    let cache = tempfile::tempdir().unwrap();
    let fetcher = TiktokenFetcher::new(client(), Some(cache.path().to_path_buf())).with_base(
        server.endpoint.clone(),
        sha256.clone(),
        sha256.clone(),
        sha256.clone(),
    );
    (server, cache, fetcher, sha256)
}

#[tokio::test]
async fn r50k_fetch_follows_read_file_cached() {
    let (server, cache, fetcher, sha256) = r50k_fixture().await;
    let expected = synthetic_bpe_file();
    let data = fetcher.data(Encoding::R50kBase).await.unwrap();
    let engine = TiktokenEngine::from_bpe_file(Encoding::R50kBase, &data).unwrap();
    assert_eq!(
        engine.encode("hello world").unwrap(),
        [256, 257, 111, 32, 119, 111, 114, 108, 100]
    );
    let url = format!("{}/encodings/r50k_base.tiktoken", server.endpoint);
    let key = hex::encode(Sha1::digest(url.as_bytes()));
    assert_eq!(fs::read(cache.path().join(key)).unwrap(), expected);
    let requests = server.request_count();
    let second = TiktokenFetcher::new(client(), Some(cache.path().to_path_buf())).with_base(
        server.endpoint.clone(),
        sha256.clone(),
        sha256.clone(),
        sha256,
    );
    let cached = second.data(Encoding::R50kBase).await.unwrap();
    let rebuilt = TiktokenEngine::from_bpe_file(Encoding::R50kBase, &cached).unwrap();
    assert_eq!(
        rebuilt.encode("hello world").unwrap(),
        [256, 257, 111, 32, 119, 111, 114, 108, 100]
    );
    assert_eq!(server.request_count(), requests);
}

#[test]
fn gpt2_vocabulary_follows_data_gym() {
    let vocab = include_bytes!("../../../fixtures/gpt2-test-vocab.bpe");
    let encoder = include_bytes!("../../../fixtures/gpt2-test-encoder.json");
    let serialized = bpe_file(vocab, encoder).unwrap();
    let engine = TiktokenEngine::from_bpe_file(Encoding::Gpt2, &serialized).unwrap();
    assert_eq!(engine.encode("hello").unwrap(), [256, 257, 78]);
    assert_eq!(engine.decode(&[256, 257, 78]).unwrap(), "hello".to_owned());
    assert_eq!(engine.encode(" t").unwrap(), [258]);
    assert_eq!(engine.decode(&[258]).unwrap(), " t".to_owned());
}

#[tokio::test]
async fn gpt2_fetch_converts_both_files() {
    let vocabulary = include_bytes!("../../../fixtures/gpt2-test-vocab.bpe");
    let encoder = include_bytes!("../../../fixtures/gpt2-test-encoder.json");
    let vocabulary_hash = hex::encode(Sha256::digest(vocabulary));
    let encoder_hash = hex::encode(Sha256::digest(encoder));
    let body = vocabulary.to_vec();
    let encoder_body = encoder.to_vec();
    let server = Server::start(move |request| {
        let (headers, data) = if request.path.ends_with("vocab.bpe") {
            (Vec::new(), body.clone())
        } else {
            (Vec::new(), encoder_body.clone())
        };
        Reply::Response {
            status: 200,
            headers,
            body: data,
        }
    })
    .await;
    let fetcher = TiktokenFetcher::new(client(), None).with_base(
        server.endpoint.clone(),
        String::new(),
        vocabulary_hash,
        encoder_hash,
    );
    let data = fetcher.data(Encoding::Gpt2).await.unwrap();
    let engine = TiktokenEngine::from_bpe_file(Encoding::Gpt2, &data).unwrap();
    assert_eq!(engine.encode(" t").unwrap(), [258]);
    assert_eq!(fetcher.data(Encoding::Gpt2).await.unwrap(), data);
    assert_eq!(server.request_count(), 2);
}
