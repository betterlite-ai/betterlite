use crate::sections::openai_like_settings::Settings;

// LiteLLM's generic Responses identity takes the caller's base or OPENAI_LIKE_API_BASE and raises without one
impl Settings {
    #[must_use]
    pub fn responses_base_required(&self, api_base: Option<&str>) -> Option<String> {
        api_base
            .filter(|explicit| !explicit.is_empty())
            .map(str::to_owned)
            .or_else(|| self.api_base.clone())
    }
}
