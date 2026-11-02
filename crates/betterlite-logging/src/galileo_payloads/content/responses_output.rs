use betterlite_models::decisions::Json;
use indexmap::IndexMap;

// openai's BaseModel allows extra fields (extra="allow"), so model_dump() writes back any key the type doesn't declare, after the declared ones, in the order the input carried them.
fn append_extra_fields(ordered: &mut IndexMap<String, Json>, fields: &IndexMap<String, Json>) {
    for (key, value) in fields {
        if !ordered.contains_key(key) {
            ordered.insert(key.clone(), value.clone());
        }
    }
}

// Builds the declared fields of a Responses-API shape in `order`, splicing the fixed type tag in wherever "type" falls in that order; a name in `order` other than "type" that's missing from `fields` invalidates the whole shape. Shared by the small Action and Caller variants whose required fields don't all come before the type tag, so required_then_typed's fixed required-then-type-then-defaulted layout doesn't fit them.
fn typed_sequence(fields: &IndexMap<String, Json>, type_tag: &str, order: &[&str]) -> Option<Json> {
    let mut ordered = IndexMap::new();
    for key in order {
        let value = if *key == "type" {
            Json::Text(type_tag.to_owned())
        } else {
            fields.get(*key)?.clone()
        };
        ordered.insert((*key).to_owned(), value);
    }
    append_extra_fields(&mut ordered, fields);
    Some(Json::Object(ordered))
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
    for key in defaulted {
        ordered.insert(
            (*key).to_owned(),
            fields.get(*key).cloned().unwrap_or(Json::Null),
        );
    }
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

// Clones `fields` and overwrites each name in `replace` with the already-projected value; shared by every shape whose own validation rewrites one or more nested fields (a content list, an action, a caller) before required_then_typed checks what else is required.
fn with_replaced(
    fields: &IndexMap<String, Json>,
    replace: &[(&str, Json)],
) -> IndexMap<String, Json> {
    let mut projected = fields.clone();
    for (key, value) in replace {
        projected.insert((*key).to_owned(), value.clone());
    }
    projected
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

// Projects ResponseOutputMessage.content (a list of output_text/refusal parts) into the shape required_then_typed can validate directly; None once a part's own type doesn't match either member.
fn message_item_project(fields: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
    Some(with_replaced(
        fields,
        &[(
            "content",
            Json::List(project_part_list(
                fields.get("content")?,
                output_content_part_dump,
            )?),
        )],
    ))
}

// Projects ResponseReasoningItem.summary and .content (lists of summary_text/reasoning_text parts) and validates .status against its Literal["in_progress", "completed", "incomplete"]; None once any of those doesn't hold.
fn reasoning_item_project(fields: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
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
    Some(with_replaced(
        fields,
        &[("summary", Json::List(summary)), ("content", content)],
    ))
}

// openai.types.responses.response_function_web_search.ActionSearch.model_dump() field order (type, queries, query, sources); sources, when present, must each validate as openai.types.responses.response_function_web_search.ActionSearchSource (type, url, both required).
fn web_search_action_search_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let Some(Json::Text(kind)) = fields.get("type") else {
        return None;
    };
    if kind != "search" {
        return None;
    }
    let sources = match fields.get("sources") {
        None | Some(Json::Null) => Json::Null,
        Some(list) => Json::List(project_part_list(list, |source| {
            match source.get("type") {
                Some(Json::Text(source_kind)) if source_kind == "url" => {
                    typed_sequence(source, "url", &["type", "url"])
                }
                _ => None,
            }
        })?),
    };
    required_then_typed(
        &with_replaced(fields, &[("sources", sources)]),
        &[],
        "search",
        &["queries", "query", "sources"],
    )
}

// openai.types.responses.response_function_web_search.ActionFind.model_dump() field order (pattern, type, url); pattern and url have no default, so a find action missing either isn't valid.
fn web_search_action_find_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    let Some(Json::Text(kind)) = fields.get("type") else {
        return None;
    };
    if kind != "find_in_page" {
        return None;
    }
    typed_sequence(fields, "find_in_page", &["pattern", "type", "url"])
}

// openai.types.responses.response_function_web_search.Action's three variants, discriminated by "type": ActionSearch, ActionOpenPage (type, url) and ActionFind; no member falls back to a plain dict, so an unrecognized or invalid action invalidates the whole web_search_call item.
fn web_search_action_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    web_search_action_search_dump(fields)
        .or_else(|| output_typed_part(fields, "open_page", &[], &["url"]))
        .or_else(|| web_search_action_find_dump(fields))
}

// Projects ResponseFunctionWebSearch.action through web_search_action_dump and validates .status against its Literal["in_progress", "searching", "completed", "failed"]; None once either doesn't hold.
fn web_search_call_project(fields: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
    let Some(Json::Text(status)) = fields.get("status") else {
        return None;
    };
    let known_status = matches!(
        status.as_str(),
        "in_progress" | "searching" | "completed" | "failed"
    );
    let Json::Object(action_fields) = fields.get("action")? else {
        return None;
    };
    if !known_status {
        return None;
    }
    let action = web_search_action_dump(action_fields)?;
    Some(with_replaced(fields, &[("action", action)]))
}

// openai.types.responses.ResponseCustomToolCall.Caller's two variants, discriminated by "type": CallerDirect ({"type": "direct"}) and CallerProgram (caller_id, type); neither falls back to a plain dict, so an unrecognized or invalid caller invalidates the whole custom_tool_call item.
fn custom_tool_caller_dump(fields: &IndexMap<String, Json>) -> Option<Json> {
    output_typed_part(fields, "direct", &[], &[])
        .or_else(|| output_typed_part(fields, "program", &["caller_id"], &[]))
}

// Projects ResponseCustomToolCall.caller through custom_tool_caller_dump; None once a present caller doesn't itself validate.
fn custom_tool_call_project(fields: &IndexMap<String, Json>) -> Option<IndexMap<String, Json>> {
    let caller = match fields.get("caller") {
        None | Some(Json::Null) => Json::Null,
        Some(Json::Object(caller_fields)) => custom_tool_caller_dump(caller_fields)?,
        Some(_) => return None,
    };
    let mut projected = fields.clone();
    projected.insert("caller".to_owned(), caller);
    Some(projected)
}

// Dispatches a Responses output item that needs its own nested fields validated and projected before required_then_typed can check the rest, by its "type" tag; kinds without a nested shape to project (function_call) stay in output_item_dump, and an unrecognized kind invalidates the item.
fn projected_item_dump(kind: &str, fields: &IndexMap<String, Json>) -> Option<Json> {
    match kind {
        // openai.types.responses.ResponseOutputMessage.model_dump() field order (id, content, role, status, type, phase); id, content, role and status have no default.
        "message" => required_then_typed(
            &message_item_project(fields)?,
            &["id", "content", "role", "status"],
            "message",
            &["phase"],
        ),
        // openai.types.responses.ResponseReasoningItem.model_dump() field order (id, summary, type, content, encrypted_content, status); id and summary have no default.
        "reasoning" => required_then_typed(
            &reasoning_item_project(fields)?,
            &["id", "summary"],
            "reasoning",
            &["content", "encrypted_content", "status"],
        ),
        // openai.types.responses.ResponseFunctionWebSearch.model_dump() field order (id, action, status, type); id, action and status have no default.
        "web_search_call" => required_then_typed(
            &web_search_call_project(fields)?,
            &["id", "action", "status"],
            "web_search_call",
            &[],
        ),
        // openai.types.responses.ResponseCustomToolCall.model_dump() field order (call_id, input, name, type, id, caller, namespace); call_id, input and name have no default.
        "custom_tool_call" => required_then_typed(
            &custom_tool_call_project(fields)?,
            &["call_id", "input", "name"],
            "custom_tool_call",
            &["id", "caller", "namespace"],
        ),
        _ => None,
    }
}

// litellm.types.llms.openai.ResponsesAPIResponse.output parses each raw item into its matching typed class when it validates; an item with an unrecognized or invalid shape stays an unmodified dict, so _serialize_galileo_output's json.dumps writes it back verbatim.
pub(super) fn output_item_dump(item: &Json) -> Json {
    let Json::Object(fields) = item else {
        return item.clone();
    };
    let projected = match fields.get("type") {
        // openai.types.responses.ResponseFunctionToolCall.model_dump() field order (arguments, call_id, name, type, id, caller, namespace, status); arguments, call_id and name have no default, so an item missing any of them isn't a valid ResponseFunctionToolCall.
        Some(Json::Text(kind)) if kind == "function_call" => required_then_typed(
            fields,
            &["arguments", "call_id", "name"],
            "function_call",
            &["id", "caller", "namespace", "status"],
        ),
        Some(Json::Text(kind)) => projected_item_dump(kind, fields),
        _ => None,
    };
    projected.unwrap_or_else(|| item.clone())
}
