use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, ExportType};

pub(super) struct Grouping {
    pub(super) key: String,
    pub(super) output: String,
    pub(super) key_alias: String,
    pub(super) user_id: String,
    pub(super) user_email: String,
    pub(super) model: String,
    pub(super) joins: String,
    pub(super) alias: String,
}

pub(super) fn export_grouping(export_type: ExportType, table: DailyActivityTable) -> Grouping {
    let (alias, entity_join) = table_parts(table);
    group_fields(export_type, alias, entity_join)
}

#[rustfmt::skip]
fn group_fields(export_type: ExportType, alias: &str, entity_join: &str) -> Grouping {
    let metadata = METADATA_JOINS;
    let metadata_join = format!("{metadata}{entity_join}");
    match export_type {
        ExportType::Daily => build(["''", "NULL::text", "NULL::text AS key_alias", "NULL::text AS user_id", "NULL::text AS user_email", "NULL::text", entity_join, alias]),
        ExportType::DailyWithKeys => build(["scoped.api_key", "NULLIF(scoped.api_key, '')", "MAX(COALESCE(vt.key_alias, dvt.key_alias)) AS key_alias", "MAX(COALESCE(vt.user_id, dvt.user_id)) AS user_id", "MAX(u.user_email) AS user_email", "NULL::text", &metadata_join, alias]),
        ExportType::DailyWithModels => build(["COALESCE(scoped.model, '')", "NULL::text", "NULL::text AS key_alias", "NULL::text AS user_id", "NULL::text AS user_email", "NULLIF(COALESCE(scoped.model, ''), '')", entity_join, alias]),
        ExportType::DailyWithUsers => build(["COALESCE(vt.user_id, dvt.user_id, '')", "NULL::text", "NULL::text AS key_alias", "MAX(COALESCE(vt.user_id, dvt.user_id)) AS user_id", "MAX(u.user_email) AS user_email", "NULL::text", &metadata_join, alias]),
    }
}

const fn table_parts(table: DailyActivityTable) -> (&'static str, &'static str) {
    match table {
        DailyActivityTable::Team => (
            "MAX(tt.team_alias)",
            " LEFT JOIN \"LiteLLM_TeamTable\" tt ON tt.team_id = scoped.team_id",
        ),
        DailyActivityTable::Organization => (
            concat!("MAX(o", "t.organization_alias)"),
            concat!(
                " LEFT JOIN \"LiteLLM_OrganizationTable\" o",
                "t ON o",
                "t.organization_id = scoped.organization_id"
            ),
        ),
        DailyActivityTable::User
        | DailyActivityTable::Tag
        | DailyActivityTable::Customer
        | DailyActivityTable::Agent => ("NULL::text", ""),
    }
}

const METADATA_JOINS: &str = " LEFT JOIN \"LiteLLM_VerificationToken\" vt ON vt.token = scoped.api_key LEFT JOIN LATERAL ( SELECT key_alias, user_id FROM \"LiteLLM_DeletedVerificationToken\" WHERE token = scoped.api_key ORDER BY deleted_at DESC LIMIT 1 ) dvt ON vt.token IS NULL LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = COALESCE(vt.user_id, dvt.user_id)";

fn build(fields: [&str; 8]) -> Grouping {
    let [
        key,
        output,
        key_alias,
        user_id,
        user_email,
        model,
        joins,
        alias,
    ] = fields;
    Grouping {
        key: key.to_owned(),
        output: output.to_owned(),
        key_alias: key_alias.to_owned(),
        user_id: user_id.to_owned(),
        user_email: user_email.to_owned(),
        model: model.to_owned(),
        joins: joins.to_owned(),
        alias: alias.to_owned(),
    }
}

pub(super) fn cursor_clause(
    scope: &DailyActivityScope,
    indexes: Option<[usize; 3]>,
    group: &str,
) -> String {
    let Some([first, second, third]) = indexes else {
        return String::new();
    };
    format!(
        " AND (scoped.date, COALESCE(scoped.\"{}\", ''), {group}) > (${first}, ${second}, ${third})",
        scope.as_ref().entity_id_field
    )
}
