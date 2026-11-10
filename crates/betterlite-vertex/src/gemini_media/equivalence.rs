use betterlite_prompts::arguments::json::Object;
#[must_use]
pub fn equivalent_key<'available>(
    key: &str,
    available: &'available Object,
) -> Option<&'available str> {
    [key.to_owned(), snake_to_camel(key), camel_to_snake(key)]
        .into_iter()
        .find_map(|candidate| {
            available
                .get_key_value(candidate.as_str())
                .map(|(stored, _)| stored.as_str())
        })
}
#[must_use]
pub fn part_exists(parts: &[Object], part: &Object, excluded_keys: &[&str]) -> bool {
    parts.iter().any(|candidate| {
        part.iter().all(|(key, value)| {
            excluded_keys.contains(&key.as_str())
                || equivalent_key(key, candidate)
                    .is_some_and(|stored| candidate.get(stored) == Some(value))
        })
    })
}
fn snake_to_camel(text: &str) -> String {
    let mut components = text.split('_');
    let mut converted = components.next().unwrap_or_default().to_owned();
    for component in components {
        let mut characters = component.chars();
        if let Some(first) = characters.next() {
            converted.extend(first.to_uppercase());
            converted.push_str(&characters.as_str().to_lowercase());
        }
    }
    converted
}
fn camel_to_snake(text: &str) -> String {
    let mut segments: Vec<String> = Vec::new();
    let mut current = String::new();
    for character in text.chars() {
        if character.is_ascii_uppercase() && !current.is_empty() {
            segments.push(current);
            current = String::new();
        }
        current.extend(character.to_lowercase());
    }
    segments.push(current);
    segments.join("_")
}
