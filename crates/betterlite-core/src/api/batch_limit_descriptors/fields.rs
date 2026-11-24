use serde::Deserialize;

// The principal wire fields this slice reads; the auth type keeps its one serializer, so they are read back
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct BatchPrincipalFields {
    pub models: Option<Vec<String>>,
    pub access_group_ids: Option<Vec<String>>,
    pub tpd_limit: Option<i64>,
    pub team_tpd_limit: Option<i64>,
    pub end_user_tpd_limit: Option<i64>,
}
