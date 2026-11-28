pub(super) const USER_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text FROM "LiteLLM_UserTable" t ORDER BY t."created_at" DESC OFFSET $1 LIMIT $2"#;
pub(super) const USER_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_UserTable""#;
pub(super) const TEAM_COUNT_SQL: &str = r#"SELECT count(*)::bigint FROM "LiteLLM_TeamTable""#;
pub(super) const ORG_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text, row_to_json(b)::text FROM "LiteLLM_OrganizationTable" t LEFT JOIN "LiteLLM_BudgetTable" b ON t."budget_id" = b."budget_id" ORDER BY t."created_at" DESC OFFSET $1 LIMIT $2"#;
pub(super) const ORG_COUNT_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_OrganizationTable""#;
pub(super) const CUSTOMER_PAGE_SQL: &str = r#"SELECT row_to_json(t)::text, row_to_json(b)::text FROM "LiteLLM_EndUserTable" t LEFT JOIN "LiteLLM_BudgetTable" b ON t."budget_id" = b."budget_id" ORDER BY t."user_id" ASC OFFSET $1 LIMIT $2"#;
pub(super) const CUSTOMER_PAGE_BUDGETED_SQL: &str = r#"SELECT row_to_json(t)::text, row_to_json(b)::text FROM "LiteLLM_EndUserTable" t LEFT JOIN "LiteLLM_BudgetTable" b ON t."budget_id" = b."budget_id" WHERE t."budget_id" IS NOT NULL ORDER BY t."user_id" ASC OFFSET $1 LIMIT $2"#;
pub(super) const CUSTOMER_COUNT_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_EndUserTable""#;
pub(super) const CUSTOMER_COUNT_BUDGETED_SQL: &str =
    r#"SELECT count(*)::bigint FROM "LiteLLM_EndUserTable" WHERE "budget_id" IS NOT NULL"#;

#[derive(Clone, Copy)]
pub(super) struct TablePages {
    pub(super) page: &'static str,
    pub(super) count: Option<&'static str>,
}

pub(super) const USER_PAGES: TablePages = TablePages {
    page: USER_PAGE_SQL,
    count: Some(USER_COUNT_SQL),
};
pub(super) const ORG_PAGES: TablePages = TablePages {
    page: ORG_PAGE_SQL,
    count: Some(ORG_COUNT_SQL),
};
