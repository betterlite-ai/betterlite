use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct JwtAuthSettings {
    pub team_id_jwt_field: Option<String>,
    pub team_ids_jwt_field: Option<String>,
    pub team_id_default: Option<String>,
    pub team_alias_jwt_field: Option<String>,
    pub org_id_jwt_field: Option<String>,
    pub org_alias_jwt_field: Option<String>,
    pub user_id_jwt_field: Option<String>,
    pub user_email_jwt_field: Option<String>,
    pub user_allowed_email_domain: Option<String>,
    pub user_id_upsert: bool,
    pub end_user_id_jwt_field: Option<String>,
    pub agent_id_jwt_field: Option<String>,
    pub object_id_jwt_field: Option<String>,
}
