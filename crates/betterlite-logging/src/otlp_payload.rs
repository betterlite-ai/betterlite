use crate::otlp_normalize::{OTLP_MAX_ATTRIBUTE_VALUE_BYTES, truncate};

mod tests;

pub fn truncate_payload(value: &str) -> String {
    if value.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES || !value.starts_with('[') {
        return truncate(value);
    }
    let Ok(messages) = serde_json::from_str::<Vec<serde_json::Value>>(value) else {
        return truncate(value);
    };
    if messages.len() < 2 {
        return truncate(value);
    }
    let encoded: Vec<String> = messages
        .iter()
        .map(|message| serde_json::to_string(message).unwrap_or_default())
        .collect();
    let marker = |count: usize| {
        serde_json::to_string(&serde_json::json!({
            "role": "system",
            "content": format!("…[{count} earlier messages truncated]")
        }))
        .unwrap_or_default()
    };
    let Some(first) = encoded.first() else {
        return truncate(value);
    };
    let marker_budget = marker(messages.len()).len().saturating_add(1);
    let budget = OTLP_MAX_ATTRIBUTE_VALUE_BYTES
        .saturating_sub(2)
        .saturating_sub(first.len())
        .saturating_sub(1)
        .saturating_sub(marker_budget);
    let tail_count = newest_that_fit(encoded.get(1..).unwrap_or_default(), budget)
        .min(messages.len().saturating_sub(2));
    if tail_count > 0 {
        let tail_start = encoded.len().saturating_sub(tail_count);
        let omitted = messages.len().saturating_sub(1).saturating_sub(tail_count);
        let mut output = vec![first.clone(), marker(omitted)];
        output.extend(encoded.iter().skip(tail_start).cloned());
        return format!("[{}]", output.join(", "));
    }
    let marker_text = marker(messages.len().saturating_sub(2));
    let half =
        (OTLP_MAX_ATTRIBUTE_VALUE_BYTES.saturating_sub(marker_text.len().saturating_add(4))) / 2;
    let first_short = shrink_message(messages.first(), half);
    let last_short = shrink_message(messages.last(), half);
    let shortened = if messages.len() > 2 {
        format!("[{first_short}, {marker_text}, {last_short}]")
    } else {
        format!("[{first_short}, {last_short}]")
    };
    if shortened.len() <= OTLP_MAX_ATTRIBUTE_VALUE_BYTES {
        shortened
    } else {
        format!("[{}]", marker(messages.len()))
    }
}

fn newest_that_fit(encoded: &[String], budget: usize) -> usize {
    let mut used: usize = 0;
    let mut count = 0;
    for message in encoded.iter().rev() {
        let next = used.saturating_add(message.len()).saturating_add(1);
        if next > budget {
            break;
        }
        used = next;
        count += 1;
    }
    count
}

fn shrink_message(message: Option<&serde_json::Value>, budget: usize) -> String {
    let Some(message) = message.and_then(serde_json::Value::as_object) else {
        return "{\"role\":\"user\"}".to_owned();
    };
    let content = message
        .get("content")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let text = content
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string(&content).unwrap_or_default());
    let role = message
        .get("role")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::String("user".to_owned()));
    let overhead = serde_json::to_string(&serde_json::json!({"role": role, "content": ""}))
        .unwrap_or_default()
        .len();
    let room = budget.saturating_sub(overhead).saturating_sub(48) / 6;
    let kept = valid_prefix(&text, room);
    serde_json::to_string(&serde_json::json!({
        "role": role,
        "content": format!("{kept}…[truncated {} bytes]", text.len().saturating_sub(kept.len()))
    }))
    .unwrap_or_else(|_| "{\"role\":\"user\"}".to_owned())
}

fn valid_prefix(value: &str, limit: usize) -> &str {
    let bytes = value.as_bytes();
    let end = limit.min(bytes.len());
    let mut boundary = end;
    while boundary > 0 && std::str::from_utf8(bytes.get(..boundary).unwrap_or_default()).is_err() {
        boundary -= 1;
    }
    std::str::from_utf8(bytes.get(..boundary).unwrap_or_default()).unwrap_or_default()
}
