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

// openai.types.responses.ResponseOutputText.model_dump() field order (annotations, text, type, logprobs); annotations and text have no default, so a part missing either isn't a valid ResponseOutputText.
fn output_text_content_part(part: &IndexMap<String, Json>) -> Option<Json> {
    let mut ordered = IndexMap::new();
    ordered.insert("annotations".to_owned(), part.get("annotations")?.clone());
    ordered.insert("text".to_owned(), part.get("text")?.clone());
    ordered.insert("type".to_owned(), Json::Text("output_text".to_owned()));
    insert_defaulted(&mut ordered, part, &["logprobs"]);
    Some(Json::Object(ordered))
}

// openai.types.responses.ResponseOutputMessage.model_dump() field order (id, content, role, status, type, phase); id, content, role and status have no default, so an item missing any of them, or whose content holds anything but output_text parts, isn't a valid ResponseOutputMessage.
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
        if !matches!(part_fields.get("type"), Some(Json::Text(kind)) if kind == "output_text") {
            return None;
        }
        projected_parts.push(output_text_content_part(part_fields)?);
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
    let mut ordered = IndexMap::new();
    ordered.insert("arguments".to_owned(), fields.get("arguments")?.clone());
    ordered.insert("call_id".to_owned(), fields.get("call_id")?.clone());
    ordered.insert("name".to_owned(), fields.get("name")?.clone());
    ordered.insert("type".to_owned(), Json::Text("function_call".to_owned()));
    insert_defaulted(
        &mut ordered,
        fields,
        &["id", "caller", "namespace", "status"],
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
        _ => None,
    };
    projected.unwrap_or_else(|| item.clone())
}
