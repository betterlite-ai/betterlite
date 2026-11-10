use indexmap::IndexMap;

pub type ToolNameMap = IndexMap<String, String>;

// LiteLLM's docstring promises Bedrock's [a-zA-Z][a-zA-Z0-9_-]* pattern, while Python's isalnum also keeps non-ASCII letters Bedrock rejects.
pub fn make_valid_bedrock_tool_name(input: &str, names: &mut ToolNameMap) -> String {
    let Some(first) = input.chars().next() else {
        return String::new();
    };
    let mut valid = String::with_capacity(input.len().saturating_add(1));
    if !first.is_ascii_alphabetic() {
        valid.push('a');
    }
    valid.extend(input.chars().map(|character| {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            character
        } else {
            '_'
        }
    }));
    if valid != input {
        names.insert(valid.clone(), input.to_owned());
    }
    valid
}
