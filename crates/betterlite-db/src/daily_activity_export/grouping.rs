use crate::daily_activity_contracts::identifiers::DailyActivityTable;
use crate::daily_activity_contracts::{DailyActivityScope, ExportType};

pub(super) struct Grouping {
    pub(super) key: &'static str,
    pub(super) output: &'static str,
    pub(super) user_fields: &'static str,
    pub(super) joins: &'static str,
}

pub(super) const fn group_fields(export_type: ExportType) -> Grouping {
    match export_type {
        ExportType::Daily => Grouping {
            key: "''",
            output: "NULL::text",
            user_fields: "NULL::text AS key_alias, NULL::text AS user_id, NULL::text AS user_email",
            joins: "",
        },
        ExportType::DailyWithKeys => Grouping {
            key: "scoped.api_key",
            output: "NULLIF(scoped.api_key, '')",
            user_fields: "MAX(COALESCE(vt.key_alias, dvt.key_alias)) AS key_alias, MAX(COALESCE(vt.user_id, dvt.user_id)) AS user_id, MAX(u.user_email) AS user_email",
            joins: METADATA_JOINS,
        },
        ExportType::DailyWithModels => Grouping {
            key: "COALESCE(scoped.model, '')",
            output: "NULL::text",
            user_fields: "NULL::text AS key_alias, NULL::text AS user_id, NULL::text AS user_email",
            joins: "",
        },
        ExportType::DailyWithUsers => Grouping {
            key: "COALESCE(vt.user_id, dvt.user_id, '')",
            output: "NULL::text",
            user_fields: "NULL::text AS key_alias, MAX(COALESCE(vt.user_id, dvt.user_id)) AS user_id, MAX(u.user_email) AS user_email",
            joins: METADATA_JOINS,
        },
    }
}

pub(super) const fn table_parts(table: DailyActivityTable) -> (&'static str, &'static str) {
    match table {
        DailyActivityTable::Team => (
            "MAX(tt.team_alias)",
            " LEFT JOIN \"LiteLLM_TeamTable\" tt ON tt.team_id = scoped.team_id",
        ),
        DailyActivityTable::Organization => (
            "MAX(ot.organization_alias)",
            " LEFT JOIN \"LiteLLM_OrganizationTable\" ot ON ot.organization_id = scoped.organization_id",
        ),
        DailyActivityTable::User
        | DailyActivityTable::Tag
        | DailyActivityTable::Customer
        | DailyActivityTable::Agent => ("NULL::text", ""),
    }
}

const METADATA_JOINS: &str = " LEFT JOIN \"LiteLLM_VerificationToken\" vt ON vt.token = scoped.api_key LEFT JOIN LATERAL ( SELECT key_alias, user_id FROM \"LiteLLM_DeletedVerificationToken\" WHERE token = scoped.api_key ORDER BY deleted_at DESC LIMIT 1 ) dvt ON vt.token IS NULL LEFT JOIN \"LiteLLM_UserTable\" u ON u.user_id = COALESCE(vt.user_id, dvt.user_id)";

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
