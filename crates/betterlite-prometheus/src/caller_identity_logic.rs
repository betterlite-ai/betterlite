pub const CALLER_IDENTITY_VALUES: [&str; 3] = ["api_key_alias", "user_email", "both"];
const METRICS: [&str; 9] = [
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
#[must_use]
pub const fn caller_identity_metrics() -> &'static [&'static str; 9] {
    &METRICS
}
#[must_use]
pub fn is_caller_identity_metric(name: &str) -> bool {
    METRICS.contains(&name)
}
pub fn validate_caller_identity(input: Option<&str>) -> Result<&'static str, String> {
    let value = input.unwrap_or("api_key_alias");
    if value == "api_key_alias" {
        Ok("api_key_alias")
    } else if value == "user_email" {
        Ok("user_email")
    } else if value == "both" {
        Ok("both")
    } else {
        Err(format!(
            "Invalid prometheus_deployment_and_latency_caller_identity={value:?}. Accepted values: {}.",
            CALLER_IDENTITY_VALUES.join(", ")
        ))
    }
}
pub fn validate_caller_identity_settings(
    input: Option<&str>,
    configs: &[(String, Vec<String>)],
) -> Result<&'static str, String> {
    let selected = validate_caller_identity(input)?;
    if selected != "user_email" {
        return Ok(selected);
    }
    let conflicts = configs
        .iter()
        .filter(|(metric, labels)| {
            is_caller_identity_metric(metric) && labels.iter().any(|label| label == "api_key_alias")
        })
        .map(|(metric, _)| metric.as_str())
        .collect::<Vec<_>>();
    if conflicts.is_empty() {
        Ok(selected)
    } else {
        Err(format!(
            "prometheus_metrics_config include_labels contains 'api_key_alias' for {}, but prometheus_deployment_and_latency_caller_identity='user_email' replaces that label on these metrics. Use 'user_email' in include_labels or change the mode.",
            conflicts.join(", ")
        ))
    }
}
pub fn resolve_caller_identity_labels(
    metric: &str,
    labels: &[String],
    input: Option<&str>,
) -> Result<Vec<String>, String> {
    let mut resolved = labels.to_vec();
    if !is_caller_identity_metric(metric) {
        return Ok(resolved);
    }
    let selected = validate_caller_identity(input)?;
    let Some(index) = resolved.iter().position(|label| label == "api_key_alias") else {
        return Ok(resolved);
    };
    if selected == "user_email" {
        if let Some(label) = resolved.get_mut(index) {
            label.clone_from(&"user_email".to_owned());
        }
    } else if selected == "both" {
        let position = index
            .checked_add(1)
            .ok_or_else(|| "caller identity label index overflow".to_owned())?;
        resolved.insert(position, "user_email".to_owned());
    } else {
        return Ok(resolved);
    }
    Ok(resolved)
}
