use crate::agentcore_request::arn::{agent_runtime_arn, region_from_arn};
use crate::agentcore_request::error::AgentCoreRequestError;
use crate::agentcore_request::tests::support::params;
use crate::agentcore_request::url::complete_url;

pub(super) fn runtime_arn_required() {
    assert_model_splitting();
    assert_region_extraction();
    assert_url_errors();
}

fn assert_model_splitting() {
    let arn = "arn:aws:bedrock-agentcore:us-west-2:123456789012:runtime/agent_runtime";
    assert_eq!(agent_runtime_arn(&format!("agentcore/{arn}")).unwrap(), arn);
    assert!(matches!(
        agent_runtime_arn("agentcore"),
        Err(AgentCoreRequestError::InvalidModelFormat)
    ));
    assert!(matches!(
        agent_runtime_arn("bedrock/agentcore/arn:aws"),
        Err(AgentCoreRequestError::InvalidModelFormat)
    ));
    assert_eq!(
        agent_runtime_arn("agentcore/arn:aws:bedrock-agentcore:us-west-2:1:runtime/a/b").unwrap(),
        "arn:aws:bedrock-agentcore:us-west-2:1:runtime/a/b",
        "the split happens once, so the runtime id keeps its slashes"
    );
}

fn assert_region_extraction() {
    let arn = "arn:aws:bedrock-agentcore:us-west-2:123456789012:runtime/agent_runtime";
    assert_eq!(region_from_arn(arn).unwrap(), "us-west-2");
    assert_eq!(region_from_arn("arn:aws:bedrock-agentcore:").unwrap(), "");
    assert_eq!(region_from_arn(":::").unwrap(), "");
    assert!(matches!(
        region_from_arn("arn:aws:bedrock-agentcore"),
        Err(AgentCoreRequestError::InvalidArn { arn: found }) if found == "arn:aws:bedrock-agentcore"
    ));
}

fn assert_url_errors() {
    let empty = params("{}");
    assert!(matches!(
        complete_url("agentcore", &empty),
        Err(AgentCoreRequestError::InvalidModelFormat)
    ));
    assert!(matches!(
        complete_url("agentcore/arn:aws", &empty),
        Err(AgentCoreRequestError::InvalidArn { arn }) if arn == "arn:aws"
    ));
    assert_eq!(
        complete_url("agentcore/not:an:arn:anywhere", &empty).unwrap(),
        "https://bedrock-agentcore.anywhere.amazonaws.com/runtimes/not%3Aan%3Aarn%3Aanywhere/invocations",
        "the remainder after the first slash is otherwise unvalidated"
    );
}
