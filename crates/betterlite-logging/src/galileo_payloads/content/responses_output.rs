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

// openai.types.responses.response_function_web_search.ActionSearchSource.model_dump() field order (type, url); both have no default, so a source missing either isn't a valid ActionSearchSource.
fn web_search_action_source_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "url" => {
            let mut ordered = IndexMap::new();
            ordered.insert("type".to_owned(), Json::Text("url".to_owned()));
            ordered.insert("url".to_owned(), fields.get("url")?.clone());
            append_extra_fields(&mut ordered, fields);
            Some(Json::Object(ordered))
        }
        _ => None,
    }
}

// openai.types.responses.response_function_web_search.ActionSearch.model_dump() field order (type, queries, query, sources); sources, when present, must each validate as ActionSearchSource.
fn web_search_action_search_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "search" => {
            let sources = match fields.get("sources") {
                None | Some(Json::Null) => Json::Null,
                Some(list) => Json::List(project_part_list(list, web_search_action_source_dump)?),
            };
            let mut with_sources = fields.clone();
            with_sources.insert("sources".to_owned(), sources);
            required_then_typed(
                &with_sources,
                &[],
                "search",
                &["queries", "query", "sources"],
            )
        }
        _ => None,
    }
}

// openai.types.responses.response_function_web_search.ActionFind.model_dump() field order (pattern, type, url); pattern and url have no default, so a find action missing either isn't valid.
fn web_search_action_find_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "find_in_page" => {
            let mut ordered = IndexMap::new();
            ordered.insert("pattern".to_owned(), fields.get("pattern")?.clone());
            ordered.insert("type".to_owned(), Json::Text("find_in_page".to_owned()));
            ordered.insert("url".to_owned(), fields.get("url")?.clone());
            append_extra_fields(&mut ordered, fields);
            Some(Json::Object(ordered))
        }
        _ => None,
    }
}

// openai.types.responses.response_function_web_search.Action's three variants, discriminated by "type": ActionSearch, ActionOpenPage, ActionFind; no member falls back to a plain dict, so an unrecognized or invalid action invalidates the whole web_search_call item.
fn web_search_action_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    web_search_action_search_dump(fields)
        .or_else(|| output_typed_part(fields, "open_page", &[], &["url"]))
        .or_else(|| web_search_action_find_dump(fields))
}

// openai.types.responses.ResponseFunctionWebSearch.model_dump() field order (id, action, status, type); id, action and status have no default. status is a Literal["in_progress", "searching", "completed", "failed"], so any other status value invalidates the item too.
fn web_search_call_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    match fields.get("status") {
        Some(Json::Text(status))
            if matches!(
                status.as_str(),
                "in_progress" | "searching" | "completed" | "failed"
            ) => {}
        _ => return None,
    }
    let Json::Object(action_fields) = fields.get("action")? else {
        return None;
    };
    let action = web_search_action_dump(action_fields)?;
    let mut with_action = fields.clone();
    with_action.insert("action".to_owned(), action);
    required_then_typed(
        &with_action,
        &["id", "action", "status"],
        "web_search_call",
        &[],
    )
}

// openai.types.responses.ResponseCustomToolCall.Caller's two variants, discriminated by "type": CallerDirect ({"type": "direct"}) and CallerProgram (caller_id, type); neither falls back to a plain dict, so an unrecognized or invalid caller invalidates the whole custom_tool_call item.
fn custom_tool_caller_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    match fields.get("type") {
        Some(Json::Text(kind)) if kind == "direct" => output_typed_part(fields, "direct", &[], &[]),
        Some(Json::Text(kind)) if kind == "program" => {
            let mut ordered = IndexMap::new();
            ordered.insert("caller_id".to_owned(), fields.get("caller_id")?.clone());
            ordered.insert("type".to_owned(), Json::Text("program".to_owned()));
            append_extra_fields(&mut ordered, fields);
            Some(Json::Object(ordered))
        }
        _ => None,
    }
}

// openai.types.responses.ResponseCustomToolCall.model_dump() field order (call_id, input, name, type, id, caller, namespace); call_id, input and name have no default, so an item missing any of them isn't a valid ResponseCustomToolCall.
fn custom_tool_call_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let caller = match fields.get("caller") {
        None | Some(Json::Null) => Json::Null,
        Some(Json::Object(caller_fields)) => custom_tool_caller_dump(caller_fields)?,
        Some(_) => return None,
    };
    let mut with_caller = fields.clone();
    with_caller.insert("caller".to_owned(), caller);
    required_then_typed(
        &with_caller,
        &["call_id", "input", "name"],
        "custom_tool_call",
        &["id", "caller", "namespace"],
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
        Some(Json::Text(kind)) if kind == "web_search_call" => web_search_call_dump(fields),
        Some(Json::Text(kind)) if kind == "custom_tool_call" => custom_tool_call_dump(fields),
        _ => None,
    };
    projected.unwrap_or_else(|| item.clone())
}
