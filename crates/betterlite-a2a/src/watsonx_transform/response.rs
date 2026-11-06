use std::collections::BTreeMap;

use crate::completion_bridge::EventSource;
use crate::completion_bridge::events::Event;
use crate::pydantic_ai_transformation::transform::ResponseMessage;
use crate::pydantic_ai_transformation::wire::Part;

mod tests;

pub fn build_a2a_message_response<S>(
    request_id: &str,
    text: &str,
    source: &mut S,
) -> Event<ResponseMessage>
where
    S: EventSource,
{
    Event::from((
        request_id.to_owned(),
        ResponseMessage {
            kind: "message",
            role: "agent",
            parts: vec![Part {
                kind: Some("text".to_owned()),
                text: Some(text.to_owned()),
                extra: BTreeMap::new(),
            }],
            message_id: source.next_uuid().to_string(),
        },
    ))
}
