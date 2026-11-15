// LiteLLM stores both Json columns as text, so each one is projected as its text value.
const PROJECTION: &str = concat!(
    "jsonb_build_object(",
    "'guardrail_id', guardrail_id, ",
    "'guardrail_name', guardrail_name, ",
    "'litellm_params', litellm_params::text, ",
    "'guardrail_info', guardrail_info::text, ",
    "'team_id', team_id, ",
    "'status', status, ",
    "'created_at', created_at, ",
    "'updated_at', updated_at",
    ")::text"
);

pub(crate) fn select_active() -> String {
    format!(
        "SELECT {PROJECTION} FROM \"LiteLLM_GuardrailsTable\" \
         WHERE status = 'active' ORDER BY created_at DESC"
    )
}

pub(crate) fn select_by_id() -> String {
    format!("SELECT {PROJECTION} FROM \"LiteLLM_GuardrailsTable\" WHERE guardrail_id = $1")
}

pub(crate) fn select_by_name() -> String {
    format!("SELECT {PROJECTION} FROM \"LiteLLM_GuardrailsTable\" WHERE guardrail_name = $1")
}

pub(crate) fn insert() -> String {
    format!(
        "INSERT INTO \"LiteLLM_GuardrailsTable\" \
         (guardrail_id, guardrail_name, litellm_params, guardrail_info, created_at, updated_at) \
         VALUES ($1, $2, to_jsonb($3::text), to_jsonb($4::text), $5, $6) RETURNING {PROJECTION}"
    )
}

pub(crate) fn update() -> String {
    format!(
        "UPDATE \"LiteLLM_GuardrailsTable\" SET guardrail_name = $2, \
         litellm_params = to_jsonb($3::text), guardrail_info = to_jsonb($4::text), updated_at = $5 \
         WHERE guardrail_id = $1 RETURNING {PROJECTION}"
    )
}

pub(crate) fn delete() -> String {
    format!(
        "DELETE FROM \"LiteLLM_GuardrailsTable\" WHERE guardrail_id = $1 RETURNING {PROJECTION}"
    )
}
