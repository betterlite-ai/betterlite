use betterlite_prompts::arguments::json::Object;

#[must_use]
pub fn equivalent_key<'available>(
    key: &str,
    available: &'available Object,
) -> Option<&'available str> {
    if let Some((stored, _)) = available.get_key_value(key) {
        return Some(stored.as_str());
    }
    let camel = snake_to_camel(key);
    if let Some((stored, _)) = available.get_key_value(&camel) {
        return Some(stored.as_str());
    }
    let snake = camel_to_snake(key);
    if let Some((stored, _)) = available.get_key_value(&snake) {
        return Some(stored.as_str());
    }
    None
}

#[must_use]
pub fn part_exists(parts: &[Object], part: &Object, excluded_keys: &[&str]) -> bool {
    for candidate in parts {
        let mut matches = true;
        for (key, value) in part {
            if excluded_keys.contains(&key.as_str()) {
                continue;
            }
            let Some(stored) = equivalent_key(key, candidate) else {
                matches = false;
                break;
            };
            if candidate.get(stored) != Some(value) {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }
    false
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
    let mut converted = String::with_capacity(text.len());
    for (index, character) in text.char_indices() {
        if index > 0 && character.is_ascii_uppercase() {
            converted.push('_');
        }
        converted.extend(character.to_lowercase());
    }
    converted
}
