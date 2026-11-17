#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use super::super::super::super::duration::billed_audio_seconds;
use super::super::super::super::route::SpeechRoute;

// Every file the fuzzer keeps replays through the short-audio and fast-transcription reply parsers.
#[rstest]
fn replays_the_azure_speech_reply_corpus(
    #[files("fuzz/corpus/azure_speech_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("a corpus file is readable");
    drop(billed_audio_seconds(SpeechRoute::ShortAudio, None, &bytes));
    drop(billed_audio_seconds(
        SpeechRoute::FastTranscription,
        None,
        &bytes,
    ));
}
