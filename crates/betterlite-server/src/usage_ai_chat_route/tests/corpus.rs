#[tokio::test]
async fn replays_the_usage_ai_chat_body_corpus() {
    assert!(replay_usage_ai_chat_body_corpus().await > 0);
}

async fn replay_usage_ai_chat_body_corpus() -> usize {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/usage_ai_chat_body");
    let entries = fs::read_dir(&root).expect("the corpus directory");
    let mut replayed = 0_usize;
    for entry in entries {
        let bytes = fs::read(entry.expect("a corpus entry").path()).expect("a corpus file");
        let model = FakeModel::new().replying(answer_reply("Hi"));
        let factory = FakeFactory::new(model, FakeSource::new());
        let request = Request::builder()
            .method(Method::POST)
            .uri("/usage/ai/chat")
            .header("content-type", "application/json")
            .body(Body::from(bytes))
            .expect("the request builds");
        let response = send(
            state(factory, None, FakeCallerId::from_subject()),
            with_principal(request, master_principal()),
        )
        .await;
        assert!(matches!(
            response.status(),
            StatusCode::OK | StatusCode::UNPROCESSABLE_ENTITY
        ));
        replayed = replayed.saturating_add(1);
    }
    replayed
}
