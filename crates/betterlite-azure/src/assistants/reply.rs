use serde_json::Value;

// The SDK fills every declared key of a nested run model, so a reply shows the fields its model dump would.
pub(super) fn sdk_shape(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let fills: [(&str, &[&str]); 3] = [
        ("tool_choice", &["function"]),
        ("incomplete_details", &["reason"]),
        ("truncation_strategy", &["last_messages"]),
    ];
    for (key, keys) in fills {
        if let Some(inner) = object.get_mut(key) {
            add_keys(inner, keys);
        }
    }
    if let Some(Value::Array(tools)) = object.get_mut("tools") {
        tools.iter_mut().for_each(shape_tool);
    }
    if let Some(format) = object.get_mut("response_format") {
        shape_format(format);
    }
}

fn add_keys(value: &mut Value, keys: &[&str]) {
    if let Some(object) = value.as_object_mut() {
        for key in keys {
            object.entry((*key).to_owned()).or_insert(Value::Null);
        }
    }
}

fn shape_tool(tool: &mut Value) {
    let Some(entry) = tool.as_object_mut() else {
        return;
    };
    match entry.get("type").and_then(Value::as_str) {
        Some("file_search") => {
            let search = entry.entry("file_search").or_insert(Value::Null);
            add_keys(search, &["max_num_results", "ranking_options"]);
            let ranking = search
                .as_object_mut()
                .and_then(|fields| fields.get_mut("ranking_options"));
            if let Some(ranking) = ranking {
                add_keys(ranking, &["ranker", "score_threshold"]);
            }
        }
        Some("function") => {
            if let Some(function) = entry.get_mut("function") {
                add_keys(function, &["description", "parameters", "strict"]);
            }
        }
        _ => {}
    }
}

fn shape_format(format: &mut Value) {
    let Some(entry) = format.as_object_mut() else {
        return;
    };
    if entry.get("type").and_then(Value::as_str) != Some("json_schema") {
        return;
    }
    let Some(schema) = entry.get_mut("json_schema") else {
        return;
    };
    add_keys(schema, &["description", "strict"]);
    if let Value::Object(fields) = schema {
        if let Some(item) = fields.remove("schema") {
            fields.insert(String::from("schema_"), item);
        }
        fields.entry(String::from("schema_")).or_insert(Value::Null);
    }
}
