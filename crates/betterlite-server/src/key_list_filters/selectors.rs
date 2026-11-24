#[derive(Debug, Clone, Copy)]
pub struct KeyListSelectors<'query> {
    pub user_id: Option<&'query str>,
    pub team_id: Option<&'query str>,
    pub organization_id: Option<&'query str>,
    // key_alias is accepted and never read, as LiteLLM's check does
    pub key_alias: Option<&'query str>,
    pub key_hash: Option<&'query str>,
}
