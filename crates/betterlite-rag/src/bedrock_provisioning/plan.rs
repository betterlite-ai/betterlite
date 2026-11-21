use serde::Serialize;

use crate::bedrock_ingestion::resources::principal::normalize_principal_arn;

pub mod index;

pub const VECTOR_DIMENSION: u32 = 1024;
pub const EF_SEARCH: u32 = 512;
pub const POLICY_VERSION: &str = "2012-10-17";

const COLLECTION: &str = "collection";
const DASHBOARD: &str = "dashboard";
const INDEX: &str = "index";
const AOSS_ALL: &str = "aoss:*";
const ALLOW: &str = "Allow";
const ASSUME_ROLE: &str = "sts:AssumeRole";
const BEDROCK_SERVICE: &str = "bedrock.amazonaws.com";
const VECTOR_TYPE: &str = "knn_vector";
const TEXT_TYPE: &str = "text";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct EncryptionPolicy {
    pub rules: Vec<CollectionRule>,
    #[serde(rename = "AWSOwnedKey")]
    pub aws_owned_key: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct CollectionRule {
    pub resource_type: String,
    pub resource: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct NetworkPolicy {
    pub rules: Vec<CollectionRule>,
    pub allow_from_public: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AccessPolicy {
    pub rules: Vec<AccessRule>,
    pub principal: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct AccessRule {
    pub resource_type: String,
    pub resource: Vec<String>,
    pub permission: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrustPolicy {
    pub version: String,
    pub statement: Vec<TrustStatement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrustStatement {
    pub effect: String,
    pub principal: ServicePrincipal,
    pub action: String,
    pub condition: TrustCondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ServicePrincipal {
    pub service: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct TrustCondition {
    pub string_equals: AccountCondition,
    pub arn_like: SourceArnCondition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountCondition {
    #[serde(rename = "aws:SourceAccount")]
    pub account: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceArnCondition {
    #[serde(rename = "aws:SourceArn")]
    pub arn: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PermissionsPolicy {
    pub version: String,
    pub statement: Vec<PermissionStatement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct PermissionStatement {
    pub effect: String,
    pub action: Vec<String>,
    pub resource: Vec<String>,
}

// the six strings an ARN the role and data policies are built from
pub struct RoleResources<'plan> {
    pub arn_prefix: &'plan str,
    pub region: &'plan str,
    pub account_id: &'plan str,
    pub embedding_model: &'plan str,
    pub collection_arn: &'plan str,
    pub bucket: &'plan str,
}

#[must_use]
pub fn encryption_policy(collection_name: &str) -> EncryptionPolicy {
    EncryptionPolicy {
        rules: vec![collection_rule(COLLECTION, collection_name)],
        aws_owned_key: true,
    }
}

#[must_use]
pub fn network_policy(collection_name: &str) -> Vec<NetworkPolicy> {
    vec![NetworkPolicy {
        rules: vec![
            collection_rule(COLLECTION, collection_name),
            collection_rule(DASHBOARD, collection_name),
        ],
        allow_from_public: true,
    }]
}

#[must_use]
pub fn data_access_policy(
    collection_name: &str,
    arn_prefix: &str,
    account_id: &str,
    caller_arn: &str,
) -> Vec<AccessPolicy> {
    let root = format!("{arn_prefix}iam::{account_id}:root");
    let caller = normalize_principal_arn(caller_arn, account_id);
    let mut principal = vec![root.clone()];
    if caller != root {
        principal.push(caller);
    }
    vec![AccessPolicy {
        rules: vec![
            AccessRule {
                resource_type: INDEX.to_owned(),
                resource: vec![format!("index/{collection_name}/*")],
                permission: vec![AOSS_ALL.to_owned()],
            },
            AccessRule {
                resource_type: COLLECTION.to_owned(),
                resource: vec![format!("collection/{collection_name}")],
                permission: vec![AOSS_ALL.to_owned()],
            },
        ],
        principal,
    }]
}

#[must_use]
pub fn trust_policy(resources: &RoleResources<'_>) -> TrustPolicy {
    TrustPolicy {
        version: POLICY_VERSION.to_owned(),
        statement: vec![TrustStatement {
            effect: ALLOW.to_owned(),
            principal: ServicePrincipal {
                service: BEDROCK_SERVICE.to_owned(),
            },
            action: ASSUME_ROLE.to_owned(),
            condition: TrustCondition {
                string_equals: AccountCondition {
                    account: resources.account_id.to_owned(),
                },
                arn_like: SourceArnCondition {
                    arn: knowledge_base_arn(resources),
                },
            },
        }],
    }
}

#[must_use]
pub fn permissions_policy(resources: &RoleResources<'_>) -> PermissionsPolicy {
    let prefix = resources.arn_prefix;
    let region = resources.region;
    let model = format!(
        "{prefix}bedrock:{region}::foundation-model/{}",
        resources.embedding_model
    );
    let bucket = format!("{prefix}s3:::{}", resources.bucket);
    let objects = format!("{bucket}/*");
    PermissionsPolicy {
        version: POLICY_VERSION.to_owned(),
        statement: vec![
            permission_statement(&["bedrock:InvokeModel"], &[model]),
            permission_statement(
                &["aoss:APIAccessAll"],
                &[resources.collection_arn.to_owned()],
            ),
            permission_statement(&["s3:GetObject", "s3:ListBucket"], &[bucket, objects]),
        ],
    }
}

fn knowledge_base_arn(resources: &RoleResources<'_>) -> String {
    format!(
        "{}bedrock:{}:{}:knowledge-base/*",
        resources.arn_prefix, resources.region, resources.account_id
    )
}

fn permission_statement(actions: &[&str], resources: &[String]) -> PermissionStatement {
    PermissionStatement {
        effect: ALLOW.to_owned(),
        action: actions.iter().map(|action| (*action).to_owned()).collect(),
        resource: resources.to_vec(),
    }
}

fn collection_rule(resource_type: &str, collection_name: &str) -> CollectionRule {
    CollectionRule {
        resource_type: resource_type.to_owned(),
        resource: vec![format!("collection/{collection_name}")],
    }
}

mod tests;
