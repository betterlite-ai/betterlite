#[must_use]
pub fn normalize_principal_arn(caller_arn: &str, account_id: &str) -> String {
    // OpenSearch data access policies want the IAM role, not the STS assumed-role session ARN
    if !caller_arn.contains(":assumed-role/") {
        return caller_arn.to_owned();
    }
    let partition = caller_arn.split(':').nth(1).unwrap_or_default();
    let role_name = caller_arn.split('/').nth(1).unwrap_or_default();
    format!("arn:{partition}:iam::{account_id}:role/{role_name}")
}

mod tests;
