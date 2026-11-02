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

// openai.types.responses.ResponseOutputMessage.model_dump() field order (id, content, role, status, type, phase); id, content, role and status have no default, so an item missing any of them, or whose content holds anything but output_text or refusal parts, isn't a valid ResponseOutputMessage.
fn output_message_item_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let id = fields.get("id")?.clone();
    let role = fields.get("role")?.clone();
    let status = fields.get("status")?.clone();
    let Some(Json::List(parts)) = fields.get("content") else {
        return None;
    };
    let mut projected_parts = Vec::with_capacity(parts.len());
    for part in parts {
        let Json::Object(part_fields) = part else {
            return None;
        };
        let projected_part = match part_fields.get("type") {
            // openai.types.responses.ResponseOutputText.model_dump() field order (annotations, text, type, logprobs); annotations and text have no default, so a part missing either isn't a valid ResponseOutputText.
            Some(Json::Text(kind)) if kind == "output_text" => required_then_typed(
                part_fields,
                &["annotations", "text"],
                "output_text",
                &["logprobs"],
            )?,
            // openai.types.responses.ResponseOutputRefusal.model_dump() field order (refusal, type); refusal has no default, so a part missing it isn't a valid ResponseOutputRefusal.
            Some(Json::Text(kind)) if kind == "refusal" => {
                required_then_typed(part_fields, &["refusal"], "refusal", &[])?
            }
            _ => return None,
        };
        projected_parts.push(projected_part);
    }
    let mut ordered = IndexMap::new();
    ordered.insert("id".to_owned(), id);
    ordered.insert("content".to_owned(), Json::List(projected_parts));
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
    let Some(Json::List(summary_items)) = fields.get("summary") else {
        return None;
    };
    let mut summary = Vec::with_capacity(summary_items.len());
    for summary_item in summary_items {
        let Json::Object(summary_fields) = summary_item else {
            return None;
        };
        summary.push(required_then_typed(
            summary_fields,
            &["text"],
            "summary_text",
            &[],
        )?);
    }
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
