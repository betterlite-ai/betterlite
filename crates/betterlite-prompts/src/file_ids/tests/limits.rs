use super::*;
use crate::arguments::json::Object;
use crate::file_ids::limits::{
    FileIdsError, MAX_CONTAINER_FILE_IDS, MAX_INPUT_CONTENT_PARTS, MAX_INPUT_ITEMS,
    MAX_MESSAGE_PARTS, MAX_MESSAGES, MAX_TOOLS, MAX_VECTOR_STORE_IDS,
};
use crate::file_ids::messages::update_messages_with_file_ids;
use crate::file_ids::responses::{
    ResponsesContent, ResponsesContentPart, ResponsesInput, ResponsesItem,
    update_responses_file_ids,
};
use crate::file_ids::tools::{ResponsesTool, decode_vector_store_ids};
use crate::message_content::{Content, ContentPart};
use rstest::rstest;
use std::iter::repeat_with;

#[derive(Clone, Copy)]
enum Collection {
    Messages,
    MessageParts,
    InputItems,
    InputParts,
    Tools,
    VectorStoreIds,
    ContainerFileIds,
}

impl Collection {
    const fn limit(self) -> usize {
        match self {
            Self::Messages => MAX_MESSAGES,
            Self::MessageParts => MAX_MESSAGE_PARTS,
            Self::InputItems => MAX_INPUT_ITEMS,
            Self::InputParts => MAX_INPUT_CONTENT_PARTS,
            Self::Tools => MAX_TOOLS,
            Self::VectorStoreIds => MAX_VECTOR_STORE_IDS,
            Self::ContainerFileIds => MAX_CONTAINER_FILE_IDS,
        }
    }
}

fn user_message(content: Option<Content>) -> Message {
    Message {
        role: "user".to_owned(),
        content,
        name: None,
        provider_specific_fields: Object::new(),
    }
}

fn messages(count: usize) -> Vec<Message> {
    repeat_with(|| user_message(None)).take(count).collect()
}

fn messages_with_parts(count: usize) -> Vec<Message> {
    let parts = repeat_with(|| ContentPart {
        kind: "text".to_owned(),
        fields: Object::new(),
    })
    .take(count)
    .collect();
    vec![user_message(Some(Content::Parts(parts)))]
}

fn input_items(count: usize) -> ResponsesInput {
    let item = ResponsesItem {
        content: None,
        fields: Object::new(),
    };
    ResponsesInput::Items(vec![item; count])
}

fn input_parts(count: usize) -> ResponsesInput {
    let parts = repeat_with(|| ResponsesContentPart {
        kind: Some("input_text".to_owned()),
        fields: Object::new(),
    })
    .take(count)
    .collect();
    ResponsesInput::Items(vec![ResponsesItem {
        content: Some(ResponsesContent::Parts(parts)),
        fields: Object::new(),
    }])
}

fn plain_tools(count: usize) -> Vec<ResponsesTool> {
    let tool = ResponsesTool {
        kind: Some("function".to_owned()),
        fields: Object::new(),
    };
    vec![tool; count]
}

fn file_search_tool(count: usize) -> Vec<ResponsesTool> {
    let ids = (0..count)
        .map(|index| Json::Text(format!("vs{index}")))
        .collect();
    vec![ResponsesTool {
        kind: Some("file_search".to_owned()),
        fields: Object::from_iter([("vector_store_ids".to_owned(), Json::List(ids))]),
    }]
}

fn code_interpreter_tool(count: usize) -> Vec<ResponsesTool> {
    let file_ids = vec!["\"file-a\""; count].join(",");
    serde_json::from_str(&format!(
        r#"[{{"type":"code_interpreter","container":{{"file_ids":[{file_ids}]}}}}]"#
    ))
    .expect("test tools decode")
}

fn run(kind: Collection, count: usize) -> Result<(), FileIdsError> {
    match kind {
        Collection::Messages => {
            let mut messages = messages(count);
            update_messages_with_file_ids(&mut messages, None, &FileIdMapping::default())
        }
        Collection::MessageParts => {
            let mut messages = messages_with_parts(count);
            update_messages_with_file_ids(&mut messages, None, &FileIdMapping::default())
        }
        Collection::InputItems => {
            update_responses_file_ids(&input_items(count), None, &FileIdMapping::default())
                .map(drop)
        }
        Collection::InputParts => {
            update_responses_file_ids(&input_parts(count), None, &FileIdMapping::default())
                .map(drop)
        }
        Collection::Tools => {
            let tools = plain_tools(count);
            decode_vector_store_ids(Some(&tools)).map(drop)
        }
        Collection::VectorStoreIds => {
            let tools = file_search_tool(count);
            decode_vector_store_ids(Some(&tools)).map(drop)
        }
        Collection::ContainerFileIds => {
            let tools = code_interpreter_tool(count);
            decode_vector_store_ids(Some(&tools)).map(drop)
        }
    }
}

#[rstest]
#[case::messages_at_the_limit(Collection::Messages, MAX_MESSAGES, true)]
#[case::messages_over_the_limit(Collection::Messages, MAX_MESSAGES.saturating_add(1), false)]
#[case::message_parts_at_the_limit(Collection::MessageParts, MAX_MESSAGE_PARTS, true)]
#[case::message_parts_over_the_limit(
    Collection::MessageParts,
    MAX_MESSAGE_PARTS.saturating_add(1),
    false
)]
#[case::input_items_at_the_limit(Collection::InputItems, MAX_INPUT_ITEMS, true)]
#[case::input_items_over_the_limit(
    Collection::InputItems,
    MAX_INPUT_ITEMS.saturating_add(1),
    false
)]
#[case::input_parts_at_the_limit(Collection::InputParts, MAX_INPUT_CONTENT_PARTS, true)]
#[case::input_parts_over_the_limit(
    Collection::InputParts,
    MAX_INPUT_CONTENT_PARTS.saturating_add(1),
    false
)]
#[case::tools_at_the_limit(Collection::Tools, MAX_TOOLS, true)]
#[case::tools_over_the_limit(Collection::Tools, MAX_TOOLS.saturating_add(1), false)]
#[case::vector_store_ids_at_the_limit(Collection::VectorStoreIds, MAX_VECTOR_STORE_IDS, true)]
#[case::vector_store_ids_over_the_limit(
    Collection::VectorStoreIds,
    MAX_VECTOR_STORE_IDS.saturating_add(1),
    false
)]
#[case::container_file_ids_at_the_limit(Collection::ContainerFileIds, MAX_CONTAINER_FILE_IDS, true)]
#[case::container_file_ids_over_the_limit(
    Collection::ContainerFileIds,
    MAX_CONTAINER_FILE_IDS.saturating_add(1),
    false
)]
fn model_file_ids_behavior_tests_behavior(
    #[case] kind: Collection,
    #[case] count: usize,
    #[case] expected_ok: bool,
) {
    let outcome = run(kind, count);
    assert_eq!(outcome.is_ok(), expected_ok);
    let Err(error) = outcome else {
        return;
    };
    let limit = kind.limit().to_string();
    assert!(error.to_string().contains(&limit), "{error}");
}
