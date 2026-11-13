use super::encoding::decode_prompt_id;

#[must_use]
pub fn id_to_repo_path(prompts_path: &str, prompt_id: &str) -> String {
    let relative = decode_prompt_id(prompt_id);
    if prompts_path.is_empty() {
        return format!("{relative}.prompt");
    }
    format!("{prompts_path}/{relative}.prompt")
}

mod tests;
