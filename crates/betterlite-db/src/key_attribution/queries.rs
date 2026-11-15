pub(crate) const ACTIVE_TOKEN_DIGEST_SQL: &str = r#"
SELECT encode(sha256(convert_to(token, 'UTF8')), 'hex') AS digest, key_alias, team_id, user_id
FROM "LiteLLM_VerificationToken"
WHERE encode(sha256(convert_to(token, 'UTF8')), 'hex') = ANY($1::text[])
"#;

pub(crate) const DELETED_TOKEN_DIGEST_SQL: &str = r#"
SELECT DISTINCT ON (token)
    encode(sha256(convert_to(token, 'UTF8')), 'hex') AS digest, key_alias, team_id, user_id
FROM "LiteLLM_DeletedVerificationToken"
WHERE encode(sha256(convert_to(token, 'UTF8')), 'hex') = ANY($1::text[])
ORDER BY token, deleted_at DESC
"#;

pub(crate) const DAILY_USER_SPEND_OWNER_SQL: &str = r#"
SELECT api_key, MIN(user_id) AS first_owner, MAX(user_id) AS last_owner
FROM "LiteLLM_DailyUserSpend"
WHERE api_key = ANY($1::text[]) AND user_id IS NOT NULL AND user_id <> ''
GROUP BY api_key
"#;
