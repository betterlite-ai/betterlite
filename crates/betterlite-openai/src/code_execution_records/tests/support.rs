#[cfg(test)]
// serde_json::json!({...}) calls Into::into to build the Map, which clippy.toml disallows, so every submodule's tests build fixtures through this type-directed helper instead
fn test_object<I>(pairs: I) -> serde_json::Value
where
    I: IntoIterator<Item = (&'static str, serde_json::Value)>,
{
    serde_json::Value::Object(serde_json::Map::from_iter(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value)),
    ))
}
