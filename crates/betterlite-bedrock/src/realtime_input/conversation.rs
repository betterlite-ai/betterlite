use super::decode::ItemCreate;
use super::frames::{
    ContentRole, ContentStart, ContentType, Event, InputConfiguration, InputFrame, ToolResult,
    ToolResultInputConfiguration, UserTextInput,
};
use crate::agentcore_stream::IdSource;
use crate::realtime_session::RealtimeSession;
use crate::realtime_session::records::{ContentEnd, TEXT_MEDIA_TYPE, TextInputConfiguration};

pub fn item_frames(
    session: &RealtimeSession,
    item: ItemCreate,
    ids: &dyn IdSource,
) -> Vec<InputFrame> {
    tracing::debug!("Handling conversation.item.create");
    match item {
        ItemCreate::Message { texts } => texts
            .into_iter()
            .flat_map(|text| text_frames(session, text, ids))
            .collect(),
        ItemCreate::ToolResult { call_id, content } => {
            Vec::from(tool_result_frames(session, call_id, content, ids))
        }
        ItemCreate::Ignored => Vec::new(),
    }
}

fn text_frames(session: &RealtimeSession, content: String, ids: &dyn IdSource) -> [InputFrame; 3] {
    let prompt_name = session.names.prompt.clone();
    let content_name = ids.next_id().to_string();
    let start = ContentStart {
        prompt_name: prompt_name.clone(),
        content_name: content_name.clone(),
        content_type: ContentType::Text,
        interactive: true,
        role: ContentRole::User,
        configuration: InputConfiguration::TextInputConfiguration(TextInputConfiguration {
            media_type: session.text_media_type.clone(),
        }),
    };
    wrap_content(
        start,
        Event::TextInput(UserTextInput {
            prompt_name,
            content_name,
            content,
        }),
    )
}

fn tool_result_frames(
    session: &RealtimeSession,
    call_id: String,
    content: String,
    ids: &dyn IdSource,
) -> [InputFrame; 3] {
    tracing::debug!("Handling conversation.item.create for tool result");
    let prompt_name = session.names.prompt.clone();
    let content_name = ids.next_id().to_string();
    let start = ContentStart {
        prompt_name: prompt_name.clone(),
        content_name: content_name.clone(),
        content_type: ContentType::Tool,
        interactive: false,
        role: ContentRole::Tool,
        configuration: InputConfiguration::ToolResultInputConfiguration(
            ToolResultInputConfiguration {
                tool_use_id: call_id,
                content_type: ContentType::Text,
                text_input_configuration: TextInputConfiguration {
                    media_type: TEXT_MEDIA_TYPE.to_owned(),
                },
            },
        ),
    };
    wrap_content(
        start,
        Event::ToolResult(ToolResult {
            prompt_name,
            content_name,
            content,
        }),
    )
}

fn wrap_content(start: ContentStart, body: Event) -> [InputFrame; 3] {
    let end = ContentEnd {
        prompt_name: start.prompt_name.clone(),
        content_name: start.content_name.clone(),
    };
    [
        InputFrame::from(Event::ContentStart(start)),
        InputFrame::from(body),
        InputFrame::from(Event::ContentEnd(end)),
    ]
}
