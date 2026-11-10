use std::slice::from_ref;

use crate::anthropic_history::thinking::drop_unsignable;
use crate::anthropic_invokes::cache_control::add_cache_control_to_content;
use crate::arguments::json::{Json, Object};

use super::{
    AssistantError, AssistantRun, REDACTED_THINKING, SERVER_TOOL_USE, TEXT, THINKING,
    TOOL_RESULT_SUFFIX, TYPE,
};

pub(super) fn append_content_blocks(
    run: &mut AssistantRun,
    blocks: &[Json],
    index: usize,
) -> Result<(), AssistantError> {
    for (position, block) in blocks.iter().enumerate() {
        let Some(fields) = block.as_object() else {
            return Err(AssistantError::BlockNotObject {
                index,
                block: position,
            });
        };
        let kind = match fields.get(TYPE) {
            None => "",
            Some(Json::Text(kind)) => kind.as_str(),
            Some(_) => {
                return Err(AssistantError::BlockField {
                    index,
                    block: position,
                    field: TYPE,
                });
            }
        };
        let text = textual_field(fields, kind, index, position)?;
        match kind {
            THINKING if !drop_unsignable(from_ref(block)).is_empty() => {
                run.blocks.push(block.clone());
            }
            TEXT => push_text(run, text, fields),
            REDACTED_THINKING | SERVER_TOOL_USE => run.blocks.push(block.clone()),
            _ if kind.ends_with(TOOL_RESULT_SUFFIX) => run.blocks.push(block.clone()),
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn push_text(run: &mut AssistantRun, text: &str, fields: &Object) {
    if !text.is_empty() {
        let element = Object::from_iter([
            (TYPE.to_owned(), Json::Text(TEXT.to_owned())),
            (TEXT.to_owned(), Json::Text(text.to_owned())),
        ]);
        run.blocks
            .push(Json::Object(add_cache_control_to_content(element, fields)));
    }
}

fn textual_field<'input>(
    fields: &'input Object,
    kind: &str,
    index: usize,
    position: usize,
) -> Result<&'input str, AssistantError> {
    let field = if kind == THINKING {
        THINKING
    } else if kind == TEXT {
        TEXT
    } else {
        return Ok("");
    };
    match fields.get(field) {
        None => Ok(""),
        Some(Json::Text(text)) => Ok(text),
        Some(_) => Err(AssistantError::BlockField {
            index,
            block: position,
            field,
        }),
    }
}
