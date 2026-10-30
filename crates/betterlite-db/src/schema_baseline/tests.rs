#![cfg(test)]

use super::{BASELINE_MIGRATION, MIGRATION_CATALOG};
use std::str;

const PINNED_BYTES: &[u8] = include_bytes!("fixtures/pinned_migration.sql");

fn sql() -> &'static str {
    str::from_utf8(BASELINE_MIGRATION.bytes).unwrap()
}

fn contains(fragments: &[&str]) {
    for fragment in fragments {
        assert!(sql().contains(fragment), "missing SQL fragment: {fragment}");
    }
}

#[test]
fn baseline_sql_identity() {
    assert_eq!(BASELINE_MIGRATION.bytes, PINNED_BYTES);
    assert_eq!(BASELINE_MIGRATION.directory, "20250326162113_baseline");
    assert_eq!(BASELINE_MIGRATION.bytes.len(), 15_827);
    assert_eq!(sql().matches("CREATE TABLE IF NOT EXISTS").count(), 17);
    let regular_indexes = sql().matches("CREATE INDEX IF NOT EXISTS").count();
    let unique_indexes = sql().matches("CREATE UNIQUE INDEX IF NOT EXISTS").count();
    assert_eq!(regular_indexes + unique_indexes, 6);
    assert_eq!(sql().matches("ADD CONSTRAINT").count(), 14);
    assert!(sql().ends_with('\n'));
}

#[test]
fn original_directory_registration() {
    assert_eq!(MIGRATION_CATALOG.files, &[BASELINE_MIGRATION]);
}

#[test]
fn statement_order_tables_indexes_foreign_keys() {
    let (before_indexes, after_indexes) = sql().split_once("-- CreateIndex").unwrap();
    let (indexes, _) = after_indexes.split_once("-- AddForeignKey").unwrap();
    assert!(before_indexes.contains("-- CreateTable"));
    assert!(!indexes.is_empty());
}

#[test]
fn seventeen_tables_six_indexes_fourteen_foreign_keys() {
    assert_eq!(sql().matches("-- CreateTable").count(), 17);
    assert_eq!(sql().matches("-- CreateIndex").count(), 6);
    assert_eq!(sql().matches("-- AddForeignKey").count(), 14);
}

macro_rules! named_case {
    ($name:ident, $($fragment:literal),+ $(,)?) => {
        #[test]
        fn $name() {
            contains(&[$($fragment),+]);
        }
    };
}

named_case!(
    baseline_defaults_budget_nullable_limits_and_required_actors,
    "\"max_budget\" DOUBLE PRECISION",
    "\"created_by\" TEXT NOT NULL"
);
named_case!(
    baseline_defaults_credentials_required_values_nullable_info,
    "\"credential_values\" JSONB NOT NULL",
    "\"credential_info\" JSONB"
);
named_case!(
    baseline_defaults_proxy_model_required_params_nullable_info,
    "\"litellm_params\" JSONB NOT NULL",
    "\"model_info\" JSONB"
);
named_case!(
    baseline_defaults_organization_json_spend_and_nullable_models,
    "\"models\" TEXT[]",
    "\"spend\" DOUBLE PRECISION NOT NULL DEFAULT 0.0"
);
named_case!(
    baseline_defaults_serial_model_id_and_nullable_aliases,
    "\"id\" SERIAL NOT NULL",
    "\"aliases\" JSONB"
);
named_case!(
    baseline_defaults_team_object_roles_false_blocked_and_nullable_arrays,
    "\"members_with_roles\" JSONB NOT NULL DEFAULT '{}'",
    "\"blocked\" BOOLEAN NOT NULL DEFAULT false"
);
named_case!(
    baseline_defaults_user_nullable_empty_arrays_and_timestamps,
    "\"teams\" TEXT[] DEFAULT ARRAY[]::TEXT[]",
    "\"created_at\" TIMESTAMP(3) DEFAULT CURRENT_TIMESTAMP"
);
named_case!(
    baseline_defaults_key_six_objects_false_cooldown_and_nullable_blocked,
    "\"soft_budget_cooldown\" BOOLEAN NOT NULL DEFAULT false",
    "\"permissions\" JSONB NOT NULL DEFAULT '{}'"
);
named_case!(
    baseline_defaults_end_user_false_blocked,
    "\"blocked\" BOOLEAN NOT NULL DEFAULT false"
);
named_case!(
    baseline_defaults_config_nullable_json,
    "\"param_value\" JSONB"
);
named_case!(
    baseline_defaults_spend_log_empty_strings_json_arrays_and_required_times,
    "\"api_key\" TEXT NOT NULL DEFAULT ''",
    "\"request_tags\" JSONB DEFAULT '[]'",
    "\"startTime\" TIMESTAMP(3) NOT NULL"
);
named_case!(
    baseline_defaults_error_log_text_status_and_required_times,
    "\"status_code\" TEXT NOT NULL DEFAULT ''",
    "\"endTime\" TIMESTAMP(3) NOT NULL"
);
named_case!(
    baseline_defaults_notification_required_status_nullable_models,
    "\"models\" TEXT[]",
    "\"status\" TEXT NOT NULL"
);
named_case!(
    baseline_defaults_team_membership_required_spend,
    "\"spend\" DOUBLE PRECISION NOT NULL DEFAULT 0.0"
);
named_case!(
    baseline_defaults_organization_membership_nullable_defaults,
    "\"spend\" DOUBLE PRECISION DEFAULT 0.0",
    "\"created_at\" TIMESTAMP(3) DEFAULT CURRENT_TIMESTAMP"
);
named_case!(
    baseline_defaults_invitation_no_timestamp_defaults,
    "\"expires_at\" TIMESTAMP(3) NOT NULL",
    "\"created_at\" TIMESTAMP(3) NOT NULL"
);
named_case!(
    baseline_defaults_audit_empty_actor_defaults,
    "\"changed_by\" TEXT NOT NULL DEFAULT ''",
    "\"changed_by_api_key\" TEXT NOT NULL DEFAULT ''"
);

#[test]
fn baseline_defaults() {
    assert!(sql().contains("CURRENT_TIMESTAMP"));
    assert!(sql().contains("DEFAULT ARRAY[]::TEXT[]"));
}

#[test]
fn baseline_catalog_columns() {
    contains(&[
        "\"budget_id\" TEXT NOT NULL",
        "\"credential_values\" JSONB NOT NULL",
        "\"id\" SERIAL NOT NULL",
        "\"members_with_roles\" JSONB NOT NULL DEFAULT '{}'",
        "\"status_code\" TEXT NOT NULL DEFAULT ''",
        "PRIMARY KEY (\"user_id\",\"team_id\")",
    ]);
}

#[test]
fn baseline_constraints() {
    contains(&[
        "CredentialsTable_credential_name_key",
        "TeamTable_model_id_key",
        "UserTable_sso_user_id_key",
        "SpendLogs_startTime_idx",
        "SpendLogs_end_user_idx",
        "OrganizationMembership_user_id_organization_id_key",
    ]);
}

#[test]
fn baseline_foreign_keys() {
    assert_eq!(sql().matches("ON UPDATE CASCADE").count(), 14);
}

macro_rules! foreign_key_case {
    ($name:ident, $constraint:literal) => {
        #[test]
        fn $name() {
            assert!(sql().contains($constraint));
        }
    };
}

foreign_key_case!(
    baseline_foreign_keys_organization_table_budget_id_fkey,
    "LiteLLM_OrganizationTable_budget_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_team_table_organization_id_fkey,
    "LiteLLM_TeamTable_organization_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_team_table_model_id_fkey,
    "LiteLLM_TeamTable_model_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_user_table_organization_id_fkey,
    "LiteLLM_UserTable_organization_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_verification_token_budget_id_fkey,
    "LiteLLM_VerificationToken_budget_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_verification_token_organization_id_fkey,
    "LiteLLM_VerificationToken_organization_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_end_user_table_budget_id_fkey,
    "LiteLLM_EndUserTable_budget_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_team_membership_budget_id_fkey,
    "LiteLLM_TeamMembership_budget_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_organization_membership_user_id_fkey,
    "LiteLLM_OrganizationMembership_user_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_organization_membership_organization_id_fkey,
    "LiteLLM_OrganizationMembership_organization_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_organization_membership_budget_id_fkey,
    "LiteLLM_OrganizationMembership_budget_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_invitation_link_user_id_fkey,
    "LiteLLM_InvitationLink_user_id_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_invitation_link_created_by_fkey,
    "LiteLLM_InvitationLink_created_by_fkey"
);
foreign_key_case!(
    baseline_foreign_keys_invitation_link_updated_by_fkey,
    "LiteLLM_InvitationLink_updated_by_fkey"
);

#[test]
fn baseline_reapply() {
    assert!(sql().contains("CREATE TABLE IF NOT EXISTS"));
    assert!(sql().contains("CREATE UNIQUE INDEX IF NOT EXISTS"));
    assert!(sql().contains("WHERE conname ="));
}

named_case!(baseline_reapply_rows_preserved, "IF NOT EXISTS");
named_case!(
    baseline_reapply_drifted_existing_table_not_repaired,
    "CREATE TABLE IF NOT EXISTS \"LiteLLM_BudgetTable\""
);
named_case!(
    baseline_reapply_foreign_key_name_in_other_schema_suppresses_creation,
    "FROM pg_constraint WHERE conname"
);

#[test]
fn baseline_reapply_independent_databases_do_not_collide() {
    assert_eq!(MIGRATION_CATALOG.files.len(), 1);
}
