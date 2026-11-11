use super::error::AgentCoreRequestError;

pub fn agent_runtime_arn(model: &str) -> Result<&str, AgentCoreRequestError> {
    let (prefix, remainder) = model
        .split_once('/')
        .ok_or(AgentCoreRequestError::InvalidModelFormat)?;
    if prefix != "agentcore" {
        return Err(AgentCoreRequestError::InvalidModelFormat);
    }
    Ok(remainder)
}

pub fn region_from_arn(arn: &str) -> Result<&str, AgentCoreRequestError> {
    // Python returns the fourth colon-separated piece even when it is empty; a shorter ARN has none
    arn.split(':')
        .nth(3)
        .ok_or_else(|| AgentCoreRequestError::InvalidArn {
            arn: arn.to_owned(),
        })
}
