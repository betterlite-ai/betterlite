#[must_use]
pub fn build_user_agent(custom_user_agent: Option<&str>, version: &str) -> String {
    if let Some(custom) = custom_user_agent {
        let trimmed = custom.trim();
        let partner = trimmed.split('/').next().unwrap_or(trimmed).trim();
        // Python asks whether the name without "_" and "-" is alphanumeric, so "__" or "-" alone is no partner
        let partner_letters: String = partner
            .chars()
            .filter(|letter| *letter != '_' && *letter != '-')
            .collect();
        if !partner_letters.is_empty() && partner_letters.chars().all(char::is_alphanumeric) {
            return format!("{partner}_litellm/{version}");
        }
    }
    format!("litellm/{version}")
}
