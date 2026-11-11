pub const ACTIVE_SQL: &str = r#"UPDATE "LiteLLM_VerificationToken" k
SET total_spend = s.sum_spend
FROM (
    SELECT k2.token, SUM(l.spend) AS sum_spend
    FROM "LiteLLM_VerificationToken" k2
    JOIN "LiteLLM_SpendLogs" l
      ON l.api_key IN (k2.token, encode(sha256(convert_to(k2.token, 'UTF8')), 'hex'))
    WHERE k2.budget_duration IS NOT NULL
       OR k2.budget_id IN (
           SELECT budget_id FROM "LiteLLM_BudgetTable" WHERE budget_duration IS NOT NULL
       )
    GROUP BY k2.token
) s
WHERE k.token = s.token
  AND k.total_spend < s.sum_spend"#;

pub const ARCHIVED_SQL: &str = r#"UPDATE "LiteLLM_DeletedVerificationToken" k
SET total_spend = s.sum_spend
FROM (
    SELECT k2.token, SUM(l.spend) AS sum_spend
    FROM (
        SELECT DISTINCT token
        FROM "LiteLLM_DeletedVerificationToken"
        WHERE budget_duration IS NOT NULL
           OR budget_id IN (
               SELECT budget_id FROM "LiteLLM_BudgetTable" WHERE budget_duration IS NOT NULL
           )
    ) k2
    JOIN "LiteLLM_SpendLogs" l
      ON l.api_key IN (k2.token, encode(sha256(convert_to(k2.token, 'UTF8')), 'hex'))
    GROUP BY k2.token
) s
WHERE k.token = s.token
  AND k.total_spend < s.sum_spend
  AND (k.budget_duration IS NOT NULL
       OR k.budget_id IN (
           SELECT budget_id FROM "LiteLLM_BudgetTable" WHERE budget_duration IS NOT NULL
       ))"#;

pub(crate) const ACTIVE_FAILURE: &str = "the active key history spend backfill update";

pub(crate) const ARCHIVED_FAILURE: &str = "the archived key history spend backfill update";

pub(crate) const STATEMENTS: [(super::BackfillStatement, &str); 2] = [
    (super::BackfillStatement::HistoryActiveKeys, ACTIVE_FAILURE),
    (
        super::BackfillStatement::HistoryArchivedKeys,
        ARCHIVED_FAILURE,
    ),
];
