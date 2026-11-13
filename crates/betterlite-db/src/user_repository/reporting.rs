use super::tables::UserFilter;

pub(crate) const SCIM_DEACTIVATED: UserFilter = UserFilter::ScimDeactivated;

pub(crate) const BACKFILL_SQL: &str = "UPDATE \"LiteLLM_UserTable\" SET \"user_email\" = $2 WHERE \"user_id\" = $1 AND \"user_email\" IS NULL";
