use serde::Serialize;
use tokio_postgres::types::ToSql;

// _find_spend_log_owners: uncapped, since a flood of colliding call ids must not push the caller's own pair out
pub const OWNERS_SQL: &str = r#"SELECT DISTINCT "user", team_id FROM "LiteLLM_SpendLogs" WHERE request_id = $1 OR litellm_call_id = $1"#;

macro_rules! payload_sql {
    ($scope:literal) => {
        concat!(
            r#"SELECT request_id, messages, response, proxy_server_request, metadata, "user", team_id FROM "LiteLLM_SpendLogs" WHERE (request_id = $1 OR litellm_call_id = $1)"#,
            $scope,
            " ORDER BY (request_id = $1) DESC LIMIT 1"
        )
    };
}

pub const ADMIN_PAYLOAD_SQL: &str = payload_sql!("");
pub const OWNER_PAYLOAD_SQL: &str = payload_sql!(r#" AND "user" = $2"#);
pub const TEAM_PAYLOAD_SQL: &str =
    payload_sql!(r#" AND ("user" = $2 OR team_id = ANY($3::text[]))"#);

// _SpendLogViewer: whose rows a non-admin id lookup may return
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendLogViewer {
    pub user_id: Option<String>,
    pub team_ids: Vec<String>,
}

// the positional binds in LiteLLM's order; a null user_id is still bound, and "user" = NULL matches nothing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum PayloadBinds<'query> {
    Admin((&'query str,)),
    Owner((&'query str, Option<&'query str>)),
    OwnerOrTeams((&'query str, Option<&'query str>, &'query [String])),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PayloadQuery<'query> {
    pub sql: &'static str,
    pub binds: PayloadBinds<'query>,
}

// _spend_log_payload_query: no viewer is an admin, and an exact request_id outranks a call-id match
#[must_use]
pub fn payload_query<'query>(
    lookup_id: &'query str,
    viewer: Option<&'query SpendLogViewer>,
) -> PayloadQuery<'query> {
    match viewer {
        None => PayloadQuery {
            sql: ADMIN_PAYLOAD_SQL,
            binds: PayloadBinds::Admin((lookup_id,)),
        },
        Some(scope) if scope.team_ids.is_empty() => PayloadQuery {
            sql: OWNER_PAYLOAD_SQL,
            binds: PayloadBinds::Owner((lookup_id, scope.user_id.as_deref())),
        },
        Some(scope) => PayloadQuery {
            sql: TEAM_PAYLOAD_SQL,
            binds: PayloadBinds::OwnerOrTeams((
                lookup_id,
                scope.user_id.as_deref(),
                scope.team_ids.as_slice(),
            )),
        },
    }
}

impl PayloadQuery<'_> {
    #[must_use]
    pub fn params(&self) -> Vec<&(dyn ToSql + Sync)> {
        let mut params: Vec<&(dyn ToSql + Sync)> = Vec::new();
        match &self.binds {
            PayloadBinds::Admin((lookup_id,)) => params.push(lookup_id),
            PayloadBinds::Owner((lookup_id, user_id)) => {
                params.push(lookup_id);
                params.push(user_id);
            }
            PayloadBinds::OwnerOrTeams((lookup_id, user_id, team_ids)) => {
                params.push(lookup_id);
                params.push(user_id);
                params.push(team_ids);
            }
        }
        params
    }
}
