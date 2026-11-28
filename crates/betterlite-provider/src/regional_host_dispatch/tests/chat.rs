use betterlite_context::chat_config_selection::tags::ChatConfigTag;

use crate::regional_host_dispatch::chat::dispatch;
use crate::regional_host_dispatch::context::CallLogging;

use super::support::{ChatFixture, ChatRecorder, logging, redactor};

#[test]
fn providers_outside_the_regional_table_have_no_arm() {
    let redactor = redactor();
    let (mut state, env) = logging(&redactor);
    let mut log = CallLogging {
        state: &mut state,
        env: &env,
    };
    let fixture = ChatFixture::new();
    let handler = ChatRecorder::succeeding();
    let context = fixture.context();
    assert!(dispatch(ChatConfigTag::DashScopeChat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::VolcEngine, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::Ai21Chat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::TencentChat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::ModelScopeChat, &context, &handler, &mut log).is_none());
    assert!(dispatch(ChatConfigTag::OpenAiGpt, &context, &handler, &mut log).is_none());
}
