use super::support;
use crate::callback_logger::CallbackLogger;
use crate::langfuse_prompt_management::INTEGRATION_NAME;

#[test]
fn the_integration_name_is_langfuse() {
    assert_eq!(INTEGRATION_NAME, "langfuse");
}

#[tokio::test]
async fn the_callback_reports_the_integration_name() {
    let variables = support::env(support::HOST);
    let fixture = support::manager(&support::borrowed(&variables)).await;
    assert_eq!(fixture.manager.callback_name(), "langfuse");
}
