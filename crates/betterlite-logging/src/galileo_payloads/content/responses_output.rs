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

// Shared by any Responses-API shape whose own model_dump is just a run of required fields (no default, so a missing one invalidates the shape), the type tag, then a run of fields defaulted to null.
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

// openai.types.responses.ResponseOutputText.model_dump() field order (annotations, text, type, logprobs) or openai.types.responses.ResponseOutputRefusal.model_dump() field order (refusal, type), picked by the part's own type tag; a part whose type is neither isn't a valid Content union member.
fn output_content_part_dump(part_fields: &IndexMap<String, Json>) -> Option<Json> {
    match part_fields.get("type") {
        Some(Json::Text(kind)) if kind == "output_text" => required_then_typed(
            part_fields,
            &["annotations", "text"],
            "output_text",
            &["logprobs"],
        ),
        Some(Json::Text(kind)) if kind == "refusal" => {
            required_then_typed(part_fields, &["refusal"], "refusal", &[])
        }
        _ => None,
    }
}

// openai.types.responses.ResponseOutputMessage.model_dump() field order (id, content, role, status, type, phase); id, content, role and status have no default, so an item missing any of them, or whose content holds anything but output_text or refusal parts, isn't a valid ResponseOutputMessage.
fn output_message_item_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let id = fields.get("id")?.clone();
    let role = fields.get("role")?.clone();
    let status = fields.get("status")?.clone();
    let content = project_part_list(fields.get("content")?, output_content_part_dump)?;
    let mut ordered = IndexMap::new();
    ordered.insert("id".to_owned(), id);
    ordered.insert("content".to_owned(), Json::List(content));
    ordered.insert("role".to_owned(), role);
    ordered.insert("status".to_owned(), status);
    ordered.insert("type".to_owned(), Json::Text("message".to_owned()));
    insert_defaulted(&mut ordered, fields, &["phase"]);
    Some(Json::Object(ordered))
}

// openai.types.responses.ResponseFunctionToolCall.model_dump() field order (arguments, call_id, name, type, id, caller, namespace, status); arguments, call_id and name have no default, so an item missing any of them isn't a valid ResponseFunctionToolCall.
fn output_function_call_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    required_then_typed(
        fields,
        &["arguments", "call_id", "name"],
        "function_call",
        &["id", "caller", "namespace", "status"],
    )
}

// openai.types.responses.ResponseReasoningItem.model_dump() field order (id, summary, type, content, encrypted_content, status); id and summary have no default, so an item missing either isn't a valid ResponseReasoningItem. Each summary entry is a Summary (text, type), with text required.
fn output_reasoning_item_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let id = fields.get("id")?.clone();
    let summary = project_part_list(fields.get("summary")?, |summary_fields| {
        required_then_typed(summary_fields, &["text"], "summary_text", &[])
    })?;
    let mut ordered = IndexMap::new();
    ordered.insert("id".to_owned(), id);
    ordered.insert("summary".to_owned(), Json::List(summary));
    ordered.insert("type".to_owned(), Json::Text("reasoning".to_owned()));
    insert_defaulted(
        &mut ordered,
        fields,
        &["content", "encrypted_content", "status"],
    );
    Some(Json::Object(ordered))
}

// litellm.types.llms.openai.ResponsesAPIResponse.output parses each raw item into its matching typed class when it validates; an item with an unrecognized or invalid shape stays an unmodified dict, so _serialize_galileo_output's json.dumps writes it back verbatim.
pub(super) fn output_item_dump(item: &Json) -> Json {
    let Json::Object(fields) = item else {
        return item.clone();
    };
    let projected = match fields.get("type") {
        Some(Json::Text(kind)) if kind == "message" => output_message_item_dump(fields),
        Some(Json::Text(kind)) if kind == "function_call" => output_function_call_dump(fields),
        Some(Json::Text(kind)) if kind == "reasoning" => output_reasoning_item_dump(fields),
        _ => None,
    };
    projected.unwrap_or_else(|| item.clone())
}
