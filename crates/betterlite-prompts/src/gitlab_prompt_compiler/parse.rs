use betterlite_models::decisions::Json as ContractJson;
use betterlite_models::prompt_contracts::AllMessageValues;
use indexmap::IndexMap;

const ROLE_PREFIXES: [(&str, &str); 3] = [
    ("system:", "system"),
    ("user:", "user"),
    ("assistant:", "assistant"),
];

#[must_use]
pub(crate) fn parse_prompt_to_messages(prompt_content: &str) -> Vec<AllMessageValues> {
    let content = prompt_content.trim();
    let mut messages = Vec::new();
    let mut current_role: Option<&str> = None;
    let mut current_content: Vec<&str> = Vec::new();
    for raw_line in content.split('\n') {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((role, rest)) = role_prefix(line) {
            if let Some(previous) = current_role.filter(|_| !current_content.is_empty()) {
                messages.push(message(previous, current_content.join("\n").trim()));
            }
            current_role = Some(role);
            current_content = vec![rest.trim()];
        } else {
            current_content.push(line);
        }
    }
    if let Some(role) = current_role.filter(|_| !current_content.is_empty()) {
        messages.push(message(role, current_content.join("\n").trim()));
    }
    if messages.is_empty() && !content.is_empty() {
        messages.push(message("user", content));
    }
    messages
}

fn role_prefix(line: &str) -> Option<(&'static str, &str)> {
    ROLE_PREFIXES.iter().find_map(|&(prefix, role)| {
        line.split_at_checked(prefix.len())
            .filter(|(head, _)| head.eq_ignore_ascii_case(prefix))
            .map(|(_, rest)| (role, rest))
    })
}

fn message(role: &str, content: &str) -> AllMessageValues {
    let mut fields = IndexMap::new();
    fields.insert("role".to_owned(), ContractJson::Text(role.to_owned()));
    fields.insert("content".to_owned(), ContractJson::Text(content.to_owned()));
    ContractJson::Object(fields)
}

mod tests;
