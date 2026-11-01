use betterlite_models::prompt_contracts::AllMessageValues;
use indexmap::IndexMap;

const ROLE_PREFIXES: [(&str, &str); 3] = [
    ("System:", "system"),
    ("User:", "user"),
    ("Assistant:", "assistant"),
];

#[must_use]
pub fn convert_to_messages(rendered_content: &str) -> Vec<AllMessageValues> {
    let content = rendered_content.trim();
    let mut messages: Vec<AllMessageValues> = Vec::new();
    let mut current_role: Option<&str> = None;
    let mut current_content: Vec<String> = Vec::new();

    for raw_line in content.split('\n') {
        let line = raw_line.trim();
        let role_match = ROLE_PREFIXES
            .iter()
            .find_map(|(prefix, role)| line.strip_prefix(prefix).map(|rest| (*role, rest)));

        if let Some((role, rest)) = role_match {
            if let Some(previous_role) = current_role {
                messages.push(create_message(
                    previous_role,
                    current_content.join("\n").trim(),
                ));
            }
            current_role = Some(role);
            current_content = vec![rest.trim().to_owned()];
        } else if current_role.is_some() {
            current_content.push(line.to_owned());
        } else {
            current_role = Some("user");
            current_content = vec![line.to_owned()];
        }
    }

    if let Some(role) = current_role {
        let trailing = current_content.join("\n");
        let trimmed = trailing.trim();
        if !trimmed.is_empty() {
            messages.push(create_message(role, trimmed));
        }
    }

    if messages.is_empty() && !content.is_empty() {
        messages.push(create_message("user", content));
    }

    messages
}

fn create_message(role: &str, content: &str) -> AllMessageValues {
    let mut message = IndexMap::new();
    message.insert("role".to_owned(), AllMessageValues::Text(role.to_owned()));
    message.insert(
        "content".to_owned(),
        AllMessageValues::Text(content.to_owned()),
    );
    AllMessageValues::Object(message)
}

mod tests;
