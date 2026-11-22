use betterlite_openai::openai_video_identifiers::ids::encode_video_id_with_provider;

use super::support::{Bench, Outcome, PROVIDER, VIDEO_ID, async_call, handled, lookups, request};

#[tokio::test]
async fn async_provider_decode_tagged_video_id_names_its_provider() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.video_id = encode_video_id_with_provider(VIDEO_ID, PROVIDER, None);
    request.custom_llm_provider = None;
    let video = async_call(&mut bench, request)
        .await
        .expect("the call answers");
    assert_eq!(video.id, VIDEO_ID);
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert_eq!(handled(&bench.journal).custom_llm_provider, PROVIDER);
}

#[tokio::test]
async fn async_provider_decode_explicit_provider_is_kept() {
    let mut bench = Bench::new(Outcome::answers());
    let mut request = request(VIDEO_ID);
    request.video_id = encode_video_id_with_provider(VIDEO_ID, "runwayml", None);
    async_call(&mut bench, request)
        .await
        .expect("the call answers");
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
    assert_eq!(handled(&bench.journal).custom_llm_provider, PROVIDER);
}
