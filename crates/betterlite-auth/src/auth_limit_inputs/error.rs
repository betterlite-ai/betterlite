#[derive(Clone, Copy, Debug, Eq, PartialEq, strum::Display)]
#[strum(serialize_all = "snake_case")]
pub enum ManagementEntity {
    Key,
    Team,
}

#[derive(Debug, thiserror::Error)]
pub enum AuthLimitInputsError {
    #[error("the deployment default limit for model {model_name} does not fit a 64-bit integer")]
    LimitOverflow { model_name: String },
    #[error(
        "Only proxy admins can set default_estimated_output_tokens or default_estimated_output_tokens_per_model on a {entity}. They decide how many output tokens the rate limiter reserves for a request that omits max_tokens."
    )]
    EstimateAdminOnly { entity: ManagementEntity },
    #[error(
        "Only proxy admins can set batch_enqueued_token_limit on a {entity}. It replaces the standard rate limit checks for batch submissions."
    )]
    BatchLimitAdminOnly { entity: ManagementEntity },
}
