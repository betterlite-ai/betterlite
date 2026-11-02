use betterlite_models::decisions::Json;
use indexmap::IndexMap;

fn insert_defaulted(
    ordered: &mut IndexMap<String, Json>,
    fields: &IndexMap<String, Json>,
    keys: &[&str],
) {
    for key in keys {
        ordered.insert(
            (*key).to_owned(),
            fields.get(*key).cloned().unwrap_or(Json::Null),
        );
    }
}

// openai's BaseModel allows extra fields (extra="allow"), so model_dump() writes back any key the type doesn't declare, after the declared ones, in the order the input carried them.
fn append_extra_fields(ordered: &mut IndexMap<String, Json>, fields: &IndexMap<String, Json>) {
    for (key, value) in fields {
        if !ordered.contains_key(key) {
            ordered.insert(key.clone(), value.clone());
        }
    }
}

// Shared by any Responses-API shape whose own model_dump is just a run of required fields (no default, so a missing one invalidates the shape), the type tag, a run of fields defaulted to null, then any field the input carried that the type doesn't declare.
fn required_then_typed(
    fields: &IndexMap<String, Json>,
    required: &[&str],
    type_tag: &str,
    defaulted: &[&str],
) -> Option<Json> {
    let mut ordered = IndexMap::new();
    for key in required {
        ordered.insert((*key).to_owned(), fields.get(*key)?.clone());
    }
    ordered.insert("type".to_owned(), Json::Text(type_tag.to_owned()));
    insert_defaulted(&mut ordered, fields, defaulted);
    append_extra_fields(&mut ordered, fields);
    Some(Json::Object(ordered))
}

// A list-typed field whose own shape invalidates the whole item unless every element parses under `project`, such as ResponseOutputMessage.content or ResponseReasoningItem.summary.
fn project_part_list<F>(list: &Json, project: F) -> Option<Vec<Json>>
where
    F: Fn(&IndexMap<String, Json>) -> Option<Json>,
{
    let Json::List(parts) = list else {
        return None;
    };
    let mut projected = Vec::with_capacity(parts.len());
    for part in parts {
        let Json::Object(part_fields) = part else {
            return None;
        };
        projected.push(project(part_fields)?);
    }
    Some(projected)
}

// Guards required_then_typed on the part's own type tag, since a part invalidates its whole list once its type doesn't match the union member being tried: shared by content parts (output_text, refusal) and ResponseReasoningItem's Summary and Content entries (summary_text, reasoning_text).
fn output_typed_part(
    part_fields: &IndexMap<String, Json>,
    type_tag: &str,
    required: &[&str],
    defaulted: &[&str],
) -> Option<Json> {
    match part_fields.get("type") {
        Some(Json::Text(kind)) if kind == type_tag => {
            required_then_typed(part_fields, required, type_tag, defaulted)
        }
        _ => None,
    }
}

// openai.types.responses.ResponseOutputText.model_dump() field order (annotations, text, type, logprobs) or openai.types.responses.ResponseOutputRefusal.model_dump() field order (refusal, type); a part whose type is neither isn't a valid Content union member.
fn output_content_part_dump(part_fields: &IndexMap<String, Json>) -> Option<Json> {
    output_typed_part(
        part_fields,
        "output_text",
        &["annotations", "text"],
        &["logprobs"],
    )
    .or_else(|| output_typed_part(part_fields, "refusal", &["refusal"], &[]))
}

// openai.types.responses.ResponseOutputMessage.model_dump() field order (id, content, role, status, type, phase); id, content, role and status have no default, so an item missing any of them, or whose content holds anything but output_text or refusal parts, isn't a valid ResponseOutputMessage.
fn output_message_item_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let content = project_part_list(fields.get("content")?, output_content_part_dump)?;
    let mut with_content = fields.clone();
    with_content.insert("content".to_owned(), Json::List(content));
    required_then_typed(
        &with_content,
        &["id", "content", "role", "status"],
        "message",
        &["phase"],
    )
}

// openai.types.responses.ResponseReasoningItem.model_dump() field order (id, summary, type, content, encrypted_content, status); id and summary have no default, so an item missing either isn't a valid ResponseReasoningItem. status is a Literal["in_progress", "completed", "incomplete"] when set, so any other status value invalidates the item too.
fn output_reasoning_item_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let summary = project_part_list(fields.get("summary")?, |part| {
        output_typed_part(part, "summary_text", &["text"], &[])
    })?;
    match fields.get("status") {
        None | Some(Json::Null) => {}
        Some(Json::Text(status))
            if matches!(status.as_str(), "in_progress" | "completed" | "incomplete") => {}
        _ => return None,
    }
    let content = match fields.get("content") {
        None | Some(Json::Null) => Json::Null,
        Some(list) => Json::List(project_part_list(list, |part| {
            output_typed_part(part, "reasoning_text", &["text"], &[])
        })?),
    };
    let mut with_summary_and_content = fields.clone();
    with_summary_and_content.insert("summary".to_owned(), Json::List(summary));
    with_summary_and_content.insert("content".to_owned(), content);
    required_then_typed(
        &with_summary_and_content,
        &["id", "summary"],
        "reasoning",
        &["content", "encrypted_content", "status"],
    )
}

// litellm.types.llms.openai.ResponsesAPIResponse.output parses each raw item into its matching typed class when it validates; an item with an unrecognized or invalid shape stays an unmodified dict, so _serialize_galileo_output's json.dumps writes it back verbatim.
pub(super) fn output_item_dump(item: &Json) -> Json {
    let Json::Object(fields) = item else {
        return item.clone();
    };
    let projected = match fields.get("type") {
        Some(Json::Text(kind)) if kind == "message" => output_message_item_dump(fields),
        // openai.types.responses.ResponseFunctionToolCall.model_dump() field order (arguments, call_id, name, type, id, caller, namespace, status); arguments, call_id and name have no default, so an item missing any of them isn't a valid ResponseFunctionToolCall.
        Some(Json::Text(kind)) if kind == "function_call" => required_then_typed(
            fields,
            &["arguments", "call_id", "name"],
            "function_call",
            &["id", "caller", "namespace", "status"],
        ),
        Some(Json::Text(kind)) if kind == "reasoning" => output_reasoning_item_dump(fields),
        _ => None,
    };
    projected.unwrap_or_else(|| item.clone())
}
