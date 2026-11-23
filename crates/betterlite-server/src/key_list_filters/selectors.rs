// the /key/list selectors validate_key_list_check reads; key_alias is accepted and never used
#[derive(Debug, Clone, Copy)]
pub struct KeyListSelectors<'query> {
    pub user_id: Option<&'query str>,
    pub team_id: Option<&'query str>,
    pub organization_id: Option<&'query str>,
    pub key_alias: Option<&'query str>,
    pub key_hash: Option<&'query str>,
}

// LiteLLM's `if value:` skips a check for None and for the empty string alike
#[must_use]
pub fn requested(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.is_empty())
}
