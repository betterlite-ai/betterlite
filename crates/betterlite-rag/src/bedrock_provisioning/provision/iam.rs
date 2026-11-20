use crate::bedrock_ingestion::client::AwsRagTransport;
use crate::bedrock_provisioning::calls::error::CallError;
use crate::bedrock_provisioning::calls::query;
use crate::bedrock_provisioning::plan;
use crate::bedrock_provisioning::plan::RoleResources;
use crate::bedrock_provisioning::provision::BedrockProvisioner;
use crate::bedrock_provisioning::provision::error::ProvisionError;

// CreateRole and PutRolePolicy stay one step; the role name is recorded between them
pub(super) async fn create_role_with_policy<Transport>(
    provisioner: &mut BedrockProvisioner<'_, Transport>,
    resources: &RoleResources<'_>,
    role_name: &str,
) -> Result<String, ProvisionError<Transport::Error>>
where
    Transport: AwsRagTransport,
{
    let trust = serde_json::to_string(&plan::trust_policy(resources)).map_err(|source| {
        CallError::Serialize {
            operation: query::CREATE_ROLE,
            source,
        }
    })?;
    let create_fields = [
        ("RoleName", role_name),
        ("AssumeRolePolicyDocument", trust.as_str()),
    ];
    let create_call = query::iam_request(resources.region, query::CREATE_ROLE, &create_fields);
    let created = provisioner
        .send(&create_call)
        .await?
        .require_success(create_call.operation)?;
    let role_arn = query::child_text(&created.body, "Role", "Arn", query::CREATE_ROLE)?;
    provisioner.state.created_resources.iam_role = Some(role_name.to_owned());
    let permissions =
        serde_json::to_string(&plan::permissions_policy(resources)).map_err(|source| {
            CallError::Serialize {
                operation: query::PUT_ROLE_POLICY,
                source,
            }
        })?;
    let policy_name = format!("{role_name}-policy");
    let attach_fields = [
        ("RoleName", role_name),
        ("PolicyName", policy_name.as_str()),
        ("PolicyDocument", permissions.as_str()),
    ];
    let attach_call = query::iam_request(resources.region, query::PUT_ROLE_POLICY, &attach_fields);
    provisioner
        .send(&attach_call)
        .await?
        .require_success(attach_call.operation)?;
    Ok(role_arn)
}
