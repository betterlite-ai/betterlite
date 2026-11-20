use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::calls::query;
use crate::bedrock_provisioning::plan;
use crate::bedrock_provisioning::plan::RoleResources;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

pub(super) async fn create_role<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    resources: &RoleResources<'_>,
    role_name: &str,
) -> Result<String, ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let document = serde_json::to_string(&plan::trust_policy(resources)).map_err(|source| {
        CallError::Serialize {
            operation: query::CREATE_ROLE,
            source,
        }
    })?;
    let body = iam_call(
        provisioner,
        query::CREATE_ROLE,
        resources.region,
        &[
            ("RoleName", role_name),
            ("AssumeRolePolicyDocument", document.as_str()),
        ],
    )
    .await?;
    Ok(query::child_text(&body, "Role", "Arn", query::CREATE_ROLE)?)
}

pub(super) async fn attach_role_policy<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    resources: &RoleResources<'_>,
    role_name: &str,
) -> Result<(), ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let document =
        serde_json::to_string(&plan::permissions_policy(resources)).map_err(|source| {
            CallError::Serialize {
                operation: query::PUT_ROLE_POLICY,
                source,
            }
        })?;
    let policy_name = format!("{role_name}-policy");
    let pairs = [
        ("RoleName", role_name),
        ("PolicyName", policy_name.as_str()),
        ("PolicyDocument", document.as_str()),
    ];
    iam_call(
        provisioner,
        query::PUT_ROLE_POLICY,
        resources.region,
        &pairs,
    )
    .await?;
    Ok(())
}

async fn iam_call<Transport>(
    provisioner: &BedrockProvisioner<'_, Transport>,
    operation: &'static str,
    region: &str,
    pairs: &[(&str, &str)],
) -> Result<Vec<u8>, ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let call = query::iam_request(region, operation, pairs);
    let response = provisioner
        .send(&call)
        .await?
        .require_success(call.operation)?;
    Ok(response.body)
}
