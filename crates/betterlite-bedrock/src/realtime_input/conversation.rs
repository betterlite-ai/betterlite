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
            .flat_map(|text| content_frames(session, ids, ItemKind::Text, text))
            .collect(),
        ItemCreate::ToolResult { call_id, content } => {
            tracing::debug!("Handling conversation.item.create for tool result");
            Vec::from(content_frames(
                session,
                ids,
                ItemKind::ToolResult(call_id),
                content,
            ))
        }
        ItemCreate::Ignored => Vec::new(),
    }
}

enum ItemKind {
    Text,
    ToolResult(String),
}

fn content_frames(
    session: &RealtimeSession,
    ids: &dyn IdSource,
    kind: ItemKind,
    content: String,
) -> [InputFrame; 3] {
    let prompt_name = session.names.prompt.clone();
    let content_name = ids.next_id().to_string();
    let (start, body) = match kind {
        ItemKind::Text => (
            ContentStart {
                prompt_name: prompt_name.clone(),
                content_name: content_name.clone(),
                content_type: ContentType::Text,
                interactive: true,
                role: ContentRole::User,
                configuration: InputConfiguration::TextInputConfiguration(TextInputConfiguration {
                    media_type: session.text_media_type.clone(),
                }),
            },
            Event::TextInput(UserTextInput {
                prompt_name: prompt_name.clone(),
                content_name: content_name.clone(),
                content,
            }),
        ),
        ItemKind::ToolResult(call_id) => (
            ContentStart {
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
            },
            Event::ToolResult(ToolResult {
                prompt_name: prompt_name.clone(),
                content_name: content_name.clone(),
                content,
            }),
        ),
    };
    let end = ContentEnd {
        prompt_name,
        content_name,
    };
    [
        InputFrame::from(Event::ContentStart(start)),
        InputFrame::from(body),
        InputFrame::from(Event::ContentEnd(end)),
    ]
}
