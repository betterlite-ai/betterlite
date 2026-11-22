use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;

use super::support::{Bench, Outcome, PROVIDER, VIDEO_ID, call, handled, lookups, ready, request};

const EMPTY_PROVIDER_ID: &str = "video_bGl0ZWxsbTpxdW90ZWQ6eyJjdXN0b21fbGxtX3Byb3ZpZGVyIjoiIiwibW9kZWxfaWQiOiJtIiwidmFsdWUiOiJ2In0=";

#[test]
fn provider_decode_tagged_video_id_names_its_provider() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.video_id = encode_video_id_with_provider(VIDEO_ID, PROVIDER, None);
    request.custom_llm_provider = None;
    ready(call(&mut bench, request));
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    let handling = handled(&bench.journal);
    assert_eq!(handling.custom_llm_provider, PROVIDER);
    assert_eq!(
        handling.video_id,
        encode_video_id_with_provider(VIDEO_ID, PROVIDER, None)
    );
}

#[test]
fn provider_decode_plain_video_id_takes_openai() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.custom_llm_provider = None;
    ready(call(&mut bench, request));
    assert_eq!(lookups(&bench.journal), vec!["openai".to_owned()]);
    assert_eq!(handled(&bench.journal).custom_llm_provider, "openai");
}

#[test]
fn provider_decode_empty_provider_in_the_id_takes_openai() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(EMPTY_PROVIDER_ID);
    request.custom_llm_provider = None;
    ready(call(&mut bench, request));
    assert_eq!(lookups(&bench.journal), vec!["openai".to_owned()]);
    assert_eq!(handled(&bench.journal).custom_llm_provider, "openai");
}

#[test]
fn provider_decode_explicit_provider_wins_over_the_id() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.video_id = encode_video_id_with_provider(VIDEO_ID, "runwayml", None);
    ready(call(&mut bench, request));
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert_eq!(handled(&bench.journal).custom_llm_provider, PROVIDER);
}
