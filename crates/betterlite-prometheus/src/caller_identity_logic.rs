pub const CALLER_IDENTITY_VALUES: [&str; 3] = ["api_key_alias", "user_email", "both"];
pub const CALLER_IDENTITY_METRICS: [&str; 9] = [
    "litellm_deployment_total_requests",
    "litellm_deployment_success_responses",
    "litellm_deployment_failure_responses",
    "litellm_request_total_latency_metric",
    "litellm_llm_api_latency_metric",
    "litellm_llm_api_time_to_first_token_metric",
    "litellm_request_queue_time_seconds",
    "litellm_overhead_latency_metric",
    "litellm_deployment_latency_per_output_token",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallerIdentity {
    ApiKeyAlias,
    UserEmail,
    Both,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CallerIdentityError {
    #[error(
        "Invalid prometheus_deployment_and_latency_caller_identity={value:?}. Accepted values: api_key_alias, user_email, both."
    )]
    InvalidMode { value: String },
    #[error(
        "prometheus_metrics_config include_labels contains 'api_key_alias' for {metrics}, but prometheus_deployment_and_latency_caller_identity='user_email' replaces that label on these metrics. Use 'user_email' in include_labels or change the mode."
    )]
    ConflictingLabels { metrics: String },
    #[error("caller identity metric {metric} has no api_key_alias label")]
    MissingAlias { metric: String },
}

pub fn validate_caller_identity(
    input: Option<&str>,
) -> Result<CallerIdentity, CallerIdentityError> {
    match input.unwrap_or("api_key_alias") {
        "api_key_alias" => Ok(CallerIdentity::ApiKeyAlias),
        "user_email" => Ok(CallerIdentity::UserEmail),
        "both" => Ok(CallerIdentity::Both),
        value => Err(CallerIdentityError::InvalidMode {
            value: value.to_owned(),
        }),
    }
}

pub fn validate_identity_settings(
    input: Option<&str>,
    configs: &[(String, Vec<String>)],
) -> Result<CallerIdentity, CallerIdentityError> {
    let selected = validate_caller_identity(input)?;
    if selected != CallerIdentity::UserEmail {
        return Ok(selected);
    }
    let conflicts = configs
        .iter()
        .filter(|(metric, labels)| {
            CALLER_IDENTITY_METRICS.contains(&metric.as_str())
                && labels.iter().any(|label| label == "api_key_alias")
        })
        .map(|(metric, _)| metric.as_str())
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        Ok(selected)
    } else {
        Err(CallerIdentityError::ConflictingLabels {
            metrics: conflicts.join(", "),
        })
    }
}

pub fn resolve_caller_identity_labels(
    metric: &str,
    labels: &[String],
    input: Option<&str>,
) -> Result<Vec<String>, CallerIdentityError> {
    if !CALLER_IDENTITY_METRICS.contains(&metric) {
        return Ok(labels.to_vec());
    }
    let selected = validate_caller_identity(input)?;
    if !labels.iter().any(|label| label == "api_key_alias") {
        return Err(CallerIdentityError::MissingAlias {
            metric: metric.to_owned(),
        });
    }
    let mut resolved = Vec::new();
    let mut replaced = false;
    for label in labels {
        if label != "api_key_alias" || replaced {
            resolved.push(label.clone());
            continue;
        }
        replaced = true;
        match selected {
            CallerIdentity::ApiKeyAlias => resolved.push(label.clone()),
            CallerIdentity::UserEmail => resolved.push("user_email".to_owned()),
            CallerIdentity::Both => {
                resolved.push(label.clone());
                resolved.push("user_email".to_owned());
            }
        }
    }
    Ok(resolved)
}
