use serde_json::Deserializer;
use serde_json::Value;

pub fn first_json_object(content: &str) -> &str {
    for (start, _) in content.match_indices('{') {
        let tail = content.get(start..).unwrap_or_default();
        let mut values = Deserializer::from_str(tail).into_iter::<Value>();
        if values.next().is_some_and(|value| value.is_ok()) {
            let end = start.saturating_add(values.byte_offset());
            return content
                .get(start..end)
                .map_or_else(|| content.trim(), |object| object);
        }
    }
    content.trim()
}
