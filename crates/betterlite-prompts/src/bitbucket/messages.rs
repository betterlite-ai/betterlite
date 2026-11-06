use betterlite_models::prompt_contracts::AllMessageValues;
use indexmap::IndexMap;

const ROLE_PREFIXES: [(&str, usize, &str); 3] = [
    ("system:", 7, "system"),
    ("user:", 5, "user"),
    ("assistant:", 10, "assistant"),
];

#[must_use]
pub fn parse_prompt_to_messages(rendered: &str) -> Vec<AllMessageValues> {
    let content = rendered.trim();
    let mut messages: Vec<AllMessageValues> = Vec::new();
    let mut current_role: Option<&str> = None;
    let mut current_content: Vec<String> = Vec::new();

    for raw_line in content.split('\n') {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        match role_prefix(line) {
            Some((role, offset)) => {
                if let Some(previous_role) = current_role {
                    messages.push(text_message(
                        previous_role,
                        current_content.join("\n").trim(),
                    ));
                }
                current_role = Some(role);
                current_content = vec![line.get(offset..).unwrap_or_default().trim().to_owned()];
            }
            None => current_content.push(line.to_owned()),
        }
    }

    if let Some(role) = current_role {
        messages.push(text_message(role, current_content.join("\n").trim()));
    }

    if messages.is_empty() && !content.is_empty() {
        messages.push(text_message("user", content));
    }

    messages
}

fn role_prefix(line: &str) -> Option<(&'static str, usize)> {
    // LiteLLM lowercases the line before matching, so a role prefix is recognized in any case
    let lowered = line.to_ascii_lowercase();
    ROLE_PREFIXES
        .iter()
        .find_map(|&(prefix, offset, role)| lowered.starts_with(prefix).then_some((role, offset)))
}

fn text_message(role: &str, content: &str) -> AllMessageValues {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), AllMessageValues::Text(role.to_owned()));
    message.insert(
        "content".to_owned(),
        AllMessageValues::Text(content.to_owned()),
    );
    AllMessageValues::Object(message)
}
