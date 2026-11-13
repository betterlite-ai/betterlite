use serde::{Deserialize, Serialize};

use crate::arguments::json::{Json, Object};
use crate::cache_placement::insert::{CACHE_CONTROL, PROMPT_CACHE_BREAKPOINT, breakpoint_marker};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum SystemPrompt {
    Text(String),
    Blocks(Vec<Object>),
}

#[must_use]
pub fn marked_system(
    system: &SystemPrompt,
    control: &Json,
    openai_dialect: bool,
) -> (SystemPrompt, bool) {
    match system {
        SystemPrompt::Text(text) => {
            let block = text_system_block(text, control, openai_dialect);
            (SystemPrompt::Blocks(vec![block]), true)
        }
        SystemPrompt::Blocks(blocks) => {
            let mut marked = blocks.clone();
            let Some(last) = marked.last_mut() else {
                return (SystemPrompt::Blocks(marked), false);
            };
            let marked_last = block_with_marker(last, control, openai_dialect);
            *last = marked_last;
            (SystemPrompt::Blocks(marked), true)
        }
    }
}

fn text_system_block(text: &str, control: &Json, openai_dialect: bool) -> Object {
    let mut block = Object::new();
    block.insert("type".to_owned(), Json::Text("text".to_owned()));
    block.insert("text".to_owned(), Json::Text(text.to_owned()));
    block_with_marker(&block, control, openai_dialect)
}

fn block_with_marker(block: &Object, control: &Json, openai_dialect: bool) -> Object {
    let mut marked = block.clone();
    if openai_dialect {
        marked.insert(PROMPT_CACHE_BREAKPOINT.to_owned(), breakpoint_marker());
    } else {
        marked.insert(CACHE_CONTROL.to_owned(), control.clone());
    }
    marked
}
