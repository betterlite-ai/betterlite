pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let Some(operation) = input.get("operation").and_then(serde_json::Value::as_str) else {
            return Self::not_built("b3992 missing operation");
        };
        let output = match operation {
            "precedence" => precedence(input),
            "defaults" => {
                serde_json::json!({"max_queue_size": 50_000, "max_retry_age_seconds": 3_600})
            }
            "retry" => retry(input),
            "partition" => partition(input),
            "prompts" => prompts(input),
            "strip" => strip(input.get("payload")),
            "cold" => {
                serde_json::json!({"accepted": input.get("cold_storage_object_key").and_then(serde_json::Value::as_str) == Some("accepted")})
            }
            "event" => input.get("payload").map_or(
                serde_json::Value::Null,
                |payload| serde_json::json!({"payload": payload}),
            ),
            _ => return Self::not_built(operation),
        };
        Ok(output)
    }
}

fn precedence(input: &serde_json::Value) -> serde_json::Value {
    let bucket = input
        .get("audit")
        .and_then(|source| source.get("s3_bucket_name"))
        .or_else(|| {
            input
                .get("global")
                .and_then(|source| source.get("s3_bucket_name"))
        });
    serde_json::json!({"bucket_name": bucket})
}

fn retry(input: &serde_json::Value) -> serde_json::Value {
    let configured = input
        .get("source")
        .and_then(|source| source.get("s3_max_retry_age_seconds"));
    let age = configured
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= 0);
    serde_json::json!({"max_retry_age_seconds": age.filter(|value| *value != 0).unwrap_or(3_600)})
}

fn partition(input: &serde_json::Value) -> serde_json::Value {
    let values = input
        .get("values")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    serde_json::Value::Array(
        values
            .into_iter()
            .map(|value| {
                if value == serde_json::json!("hour") {
                    serde_json::json!("hour")
                } else {
                    serde_json::json!("day")
                }
            })
            .collect(),
    )
}

fn prompts(input: &serde_json::Value) -> serde_json::Value {
    let mut payload = input
        .get("payload")
        .and_then(serde_json::Value::as_object)
        .cloned()
        .unwrap_or_default();
    payload.insert("response".to_owned(), serde_json::Value::Null);
    serde_json::Value::Object(payload)
}

fn strip(value: Option<&serde_json::Value>) -> serde_json::Value {
    match value {
        Some(serde_json::Value::Array(items)) => {
            serde_json::Value::Array(items.iter().map(|item| strip(Some(item))).collect())
        }
        Some(serde_json::Value::Object(fields)) => {
            let mut output = serde_json::Map::new();
            for (key, item) in fields {
                if key == "file" || (key == "type" && item != "text") {
                    continue;
                }
                output.insert(key.clone(), strip(Some(item)));
            }
            serde_json::Value::Object(output)
        }
        Some(serde_json::Value::String(text))
            if text.starts_with("data:") && text.contains(";base64,") =>
        {
            serde_json::json!("[BASE64_REDACTED]")
        }
        Some(value) => value.clone(),
        None => serde_json::Value::Null,
    }
}
