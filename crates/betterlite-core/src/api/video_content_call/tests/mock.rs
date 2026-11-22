use super::support::{
    Bench, Outcome, PROVIDER, VIDEO_ID, call, handled, kwargs, lookups, ready, request, text,
};

const MOCK_JSON: &str = r#"{"id":"mock-1","object":"video","status":"completed"}"#;

// video_content has no mock branch: the mock_response video_status answers with still reaches the provider
#[test]
fn mock_response_content_reaches_the_handler_instead_of_answering() {
    let mut bench = Bench::new(Outcome::Bytes(b"provider-bytes".to_vec()));
    let mut request = request(VIDEO_ID);
    request.kwargs = kwargs(&[("mock_response", text(MOCK_JSON))]);
    assert_eq!(ready(call(&mut bench, request)), b"provider-bytes".to_vec());
    let handling = handled(&bench.journal);
    assert_eq!(
        handling.litellm_params.get("mock_response"),
        Some(&text(MOCK_JSON))
    );
    assert_eq!(lookups(&bench.journal), vec![PROVIDER.to_owned()]);
}
