pub const ACTIVE_SQL: &str =
    r#"UPDATE "LiteLLM_VerificationToken" SET total_spend = spend WHERE total_spend < spend"#;

pub const ARCHIVED_SQL: &str = r#"UPDATE "LiteLLM_DeletedVerificationToken" SET total_spend = spend WHERE total_spend < spend"#;

pub(crate) const ACTIVE_FAILURE: &str = "the active key lifetime spend backfill update";

pub(crate) const ARCHIVED_FAILURE: &str = "the archived key lifetime spend backfill update";

pub(crate) const STATEMENTS: [(super::BackfillStatement, &str); 2] = [
    (super::BackfillStatement::FloorActiveKeys, ACTIVE_FAILURE),
    (
        super::BackfillStatement::FloorArchivedKeys,
        ARCHIVED_FAILURE,
    ),
];
