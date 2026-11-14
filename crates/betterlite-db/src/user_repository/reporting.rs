pub(crate) const USER_COUNT_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\"";

pub(crate) const USER_SCIM_DEACTIVATED_SQL: &str = "SELECT count(*) FROM \"LiteLLM_UserTable\" WHERE \"metadata\" -> 'scim_active' = 'false'::jsonb";

pub(crate) const BACKFILL_SQL: &str = "UPDATE \"LiteLLM_UserTable\" SET \"user_email\" = $2 WHERE \"user_id\" = $1 AND \"user_email\" IS NULL";
