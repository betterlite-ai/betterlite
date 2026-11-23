use indexmap::IndexMap;
use secrecy::SecretString;

use crate::speech::binary::{AudioSpeechCall, finish_audio_speech};
use crate::speech::dispatch::SpeechGlobals;

use super::support::{self, API_BASE, Inputs, Recorder, SECRET, json};

// Two credentials, or one credential in its sync and async form, never share a client entry or its headers.
#[test]
fn azure_speech_credential_identity_isolation() {
    let mut inputs = Inputs::default();
    let first = credentialed("sk-first-key", API_BASE);
    let first_request = support::start(&mut inputs, &first, &mut Recorder::default())
        .expect("the first request builds");
    let second = credentialed("sk-second-key", API_BASE);
    let second_request = support::start(&mut inputs, &second, &mut Recorder::default())
        .expect("the second request builds");
    assert_eq!(
        support::header(&first_request, "api-key"),
        Some("sk-first-key")
    );
    assert_eq!(
        support::header(&second_request, "api-key"),
        Some("sk-second-key")
    );
    assert_eq!(inputs.cache.entries.len(), 2);
    assert_ne!(inputs.cache.entries[0].0, inputs.cache.entries[1].0);
    let _repeat = support::start(&mut inputs, &first, &mut Recorder::default())
        .expect("the repeat request builds");
    assert_eq!(inputs.cache.entries.len(), 2);
    let moved = credentialed("sk-first-key", "https://other.openai.azure.com");
    let moved_request = support::start(&mut inputs, &moved, &mut Recorder::default())
        .expect("the moved request builds");
    assert_eq!(moved_request.url.host_str(), Some("other.openai.azure.com"));
    assert_eq!(
        support::header(&moved_request, "api-key"),
        Some("sk-first-key")
    );
    assert_eq!(inputs.cache.entries.len(), 2);
    let mut asynchronous = credentialed("sk-first-key", API_BASE);
    asynchronous.aspeech = true;
    let asynchronous_request = support::start(&mut inputs, &asynchronous, &mut Recorder::default())
        .expect("the async request builds");
    assert_eq!(
        support::header(&asynchronous_request, "api-key"),
        Some("sk-first-key")
    );
    assert_eq!(inputs.cache.entries.len(), 3);
}

#[test]
fn azure_speech_secret_safe_failure() {
    let globals = SpeechGlobals {
        api_base: Some(String::from("https://x.example")),
        api_key: Some(SecretString::from(SECRET)),
        azure_key: Some(SecretString::from(SECRET)),
    };
    assert!(!format!("{globals:?}").contains(SECRET));
    let mut call = support::call(
        "tts",
        "hello",
        "alloy",
        support::pairs(&[("extra_headers", json(r#"{"x-trace":"bad\nvalue"}"#))]),
    );
    call.api_key = Some(SecretString::from(SECRET));
    let error = support::start(&mut Inputs::default(), &call, &mut Recorder::default())
        .expect_err("an invalid header value fails the request");
    let (class, _status) = error.kind();
    assert_eq!(class, "Exception");
    assert!(!format!("{error:?} {error}").contains(SECRET));
    let Err(failed) = finish_audio_speech(support::response(401, &[], b"denied")) else {
        panic!("the status fails the call");
    };
    let (failed_class, _failed_status) = failed.kind();
    assert_eq!(failed_class, "APIStatusError");
    let shown = format!("{failed:?} {failed}");
    assert!(!shown.contains(SECRET));
    assert!(shown.contains("denied"));
}

fn credentialed(key: &'static str, api_base: &'static str) -> AudioSpeechCall<'static> {
    let mut call = support::call("tts", "hello", "alloy", IndexMap::new());
    call.api_key = Some(SecretString::from(key));
    call.api_base = Some(String::from(api_base));
    call
}
