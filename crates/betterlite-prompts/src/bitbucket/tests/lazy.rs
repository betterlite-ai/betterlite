use crate::support;

#[tokio::test]
async fn lazy_prompt_manager_is_built_once_and_reused() {
    let server = support::Server::start(&[("greeting.prompt", 200, "Hi {{ name }}")]);
    let manager = support::manager(
        support::config(&server.base_url),
        Some("greeting"),
        support::transport(),
    );

    let first = manager
        .prompt_manager()
        .await
        .expect("first access builds the template manager");
    assert!(first.prompts.contains_key("greeting"));
    drop(first);

    let second = manager
        .prompt_manager()
        .await
        .expect("second access returns the built manager");
    assert!(second.prompts.contains_key("greeting"));
    drop(second);

    assert_eq!(
        server.request_count(),
        1,
        "the prompt is fetched once, at construction"
    );
}

#[tokio::test]
async fn lazy_construction_does_not_fetch_without_a_prompt_id() {
    let manager = support::manager(
        support::config(support::CLOSED_BASE),
        None,
        support::transport(),
    );

    let guard = manager
        .prompt_manager()
        .await
        .expect("the closed port is never dialed when no prompt is configured");
    assert!(guard.prompts.is_empty());
    drop(guard);
}
