#[derive(Clone, Copy, Debug, Default)]
pub struct PrincipalIdentity<'identity> {
    // LiteLLM's `if user_api_key_dict.api_key:` treats an empty claim as no api_key at all
    pub api_key: Option<&'identity str>,
    pub user_id: Option<&'identity str>,
    pub team_id: Option<&'identity str>,
    pub org_id: Option<&'identity str>,
}
