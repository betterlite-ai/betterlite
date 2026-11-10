use betterlite_prompts::arguments::json::{Json, Object};

// LiteLLM rewrites the caller's assistant history in place, so the messages go in mutably.
pub fn strip_advisor_blocks(messages: &mut [Json], replace_with_text: bool) {
    for message in messages.iter_mut() {
        let Json::Object(fields) = message else {
            continue;
        };
        if fields.get("role").and_then(Json::as_str) != Some("assistant") {
            continue;
        }
        let Some(Json::List(blocks)) = fields.get("content") else {
            continue;
        };
        let Some(content) = cleaned_content(blocks, replace_with_text) else {
            continue;
        };
        fields.insert("content".to_owned(), content);
    }
}

fn cleaned_content(blocks: &[Json], replace_with_text: bool) -> Option<Json> {
    let mut advisor_ids = collected_ids(blocks);
    if advisor_ids.is_empty() {
        return None;
    }
    if replace_with_text {
        collect_advice(blocks, &mut advisor_ids);
    }
    Some(Json::List(kept_blocks(
        blocks,
        &advisor_ids,
        replace_with_text,
    )))
}

fn collected_ids(blocks: &[Json]) -> Vec<(Json, Option<String>)> {
    let mut advisor_ids = Vec::new();
    for block in blocks {
        let Some(fields) = block.as_object() else {
            continue;
        };
        if !is_advisor_use(fields) {
            continue;
        }
        let Some(id) = fields.get("id").filter(|id| id.is_truthy()) else {
            continue;
        };
        if advisor_ids.iter().all(|(known, _)| known != id) {
            advisor_ids.push((id.clone(), None));
        }
    }
    advisor_ids
}

fn collect_advice(blocks: &[Json], advisor_ids: &mut [(Json, Option<String>)]) {
    for block in blocks {
        let Some(fields) = block.as_object() else {
            continue;
        };
        if fields.get("type").and_then(Json::as_str) != Some("advisor_tool_result") {
            continue;
        }
        let Some(index) = fields.get("tool_use_id").and_then(|tool_use_id| {
            advisor_ids
                .iter()
                .position(|(known, _)| known == tool_use_id)
        }) else {
            continue;
        };
        if let Some(entry) = advisor_ids.get_mut(index) {
            entry.1 = Some(advisor_text(fields.get("content")));
        }
    }
}

fn kept_blocks(
    blocks: &[Json],
    advisor_ids: &[(Json, Option<String>)],
    replace_with_text: bool,
) -> Vec<Json> {
    let mut kept = Vec::with_capacity(blocks.len());
    for block in blocks {
        let Some(fields) = block.as_object() else {
            kept.push(block.clone());
            continue;
        };
        let is_use = is_advisor_use(fields) && has_advisor_id(fields, "id", advisor_ids);
        if is_use {
            let advice = if replace_with_text {
                advisor_advice(fields, advisor_ids)
            } else {
                None
            };
            if let Some(text) = advice.filter(|text| !text.is_empty()) {
                kept.push(advisor_feedback(text));
            }
            continue;
        }
        let is_result = fields.get("type").and_then(Json::as_str) == Some("advisor_tool_result")
            && has_advisor_id(fields, "tool_use_id", advisor_ids);
        if is_result {
            continue;
        }
        kept.push(block.clone());
    }
    kept
}

fn is_advisor_use(fields: &Object) -> bool {
    fields.get("type").and_then(Json::as_str) == Some("server_tool_use")
        && fields.get("name").and_then(Json::as_str) == Some("advisor")
}

fn has_advisor_id(fields: &Object, key: &str, advisor_ids: &[(Json, Option<String>)]) -> bool {
    fields
        .get(key)
        .is_some_and(|id| advisor_ids.iter().any(|(known, _)| known == id))
}

fn advisor_advice<'ids>(
    fields: &Object,
    advisor_ids: &'ids [(Json, Option<String>)],
) -> Option<&'ids str> {
    let index = fields
        .get("id")
        .and_then(|id| advisor_ids.iter().position(|(known, _)| known == id))?;
    let (_, text) = advisor_ids.get(index)?;
    text.as_deref()
}

// LiteLLM keeps the first text block of an advisor result and takes its advice verbatim.
fn advisor_text(content: Option<&Json>) -> String {
    if let Some(Json::Text(text)) = content {
        return text.clone();
    }
    let Some(Json::List(blocks)) = content else {
        return String::new();
    };
    first_text_block(blocks)
}

fn first_text_block(blocks: &[Json]) -> String {
    let text_block = blocks
        .iter()
        .filter_map(Json::as_object)
        .find(|block| block.get("type").and_then(Json::as_str) == Some("text"));
    text_block
        .and_then(|block| block.get("text").and_then(Json::as_str))
        .unwrap_or_default()
        .to_owned()
}

fn advisor_feedback(advice: &str) -> Json {
    Json::Object(Object::from_iter([
        ("type".to_owned(), Json::Text("text".to_owned())),
        (
            "text".to_owned(),
            Json::Text(format!("<advisor_feedback>\n{advice}\n</advisor_feedback>")),
        ),
    ]))
}
