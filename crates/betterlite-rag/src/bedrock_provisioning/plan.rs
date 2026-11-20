use serde::Serialize;

use crate::bedrock_ingestion::resources::principal::normalize_principal_arn;

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
pub struct IndexBody {
    pub settings: IndexSettings,
    pub mappings: IndexMappings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexSettings {
    pub index: IndexKnobs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexKnobs {
    pub knn: bool,
    #[serde(rename = "knn.algo_param.ef_search")]
    pub ef_search: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexMappings {
    pub properties: IndexProperties,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexProperties {
    #[serde(rename = "bedrock-knowledge-base-default-vector")]
    pub vector: VectorField,
    #[serde(rename = "AMAZON_BEDROCK_METADATA")]
    pub metadata: MetadataField,
    #[serde(rename = "AMAZON_BEDROCK_TEXT_CHUNK")]
    pub text_chunk: TextField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorField {
    #[serde(rename = "type")]
    pub field_type: String,
    pub dimension: u32,
    pub method: VectorMethod,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VectorMethod {
    pub engine: String,
    pub name: String,
    pub space_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MetadataField {
    #[serde(rename = "type")]
    pub field_type: String,
    pub index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TextField {
    #[serde(rename = "type")]
    pub field_type: String,
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
pub fn index_body() -> IndexBody {
    IndexBody {
        settings: IndexSettings {
            index: IndexKnobs {
                knn: true,
                ef_search: EF_SEARCH,
            },
        },
        mappings: IndexMappings {
            properties: IndexProperties {
                vector: VectorField {
                    field_type: VECTOR_TYPE.to_owned(),
                    dimension: VECTOR_DIMENSION,
                    method: VectorMethod {
                        engine: "faiss".to_owned(),
                        name: "hnsw".to_owned(),
                        space_type: "l2".to_owned(),
                    },
                },
                metadata: MetadataField {
                    field_type: TEXT_TYPE.to_owned(),
                    index: false,
                },
                text_chunk: TextField {
                    field_type: TEXT_TYPE.to_owned(),
                },
            },
        },
    }
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
                    arn: format!(
                        "{}bedrock:{}:{}:knowledge-base/*",
                        resources.arn_prefix, resources.region, resources.account_id
                    ),
                },
            },
        }],
    }
}

#[must_use]
pub fn permissions_policy(resources: &RoleResources<'_>) -> PermissionsPolicy {
    let prefix = resources.arn_prefix;
    let region = resources.region;
    PermissionsPolicy {
        version: POLICY_VERSION.to_owned(),
        statement: vec![
            PermissionStatement {
                effect: ALLOW.to_owned(),
                action: vec!["bedrock:InvokeModel".to_owned()],
                resource: vec![format!(
                    "{prefix}bedrock:{region}::foundation-model/{}",
                    resources.embedding_model
                )],
            },
            PermissionStatement {
                effect: ALLOW.to_owned(),
                action: vec!["aoss:APIAccessAll".to_owned()],
                resource: vec![resources.collection_arn.to_owned()],
            },
            PermissionStatement {
                effect: ALLOW.to_owned(),
                action: vec!["s3:GetObject".to_owned(), "s3:ListBucket".to_owned()],
                resource: vec![
                    format!("{prefix}s3:::{}", resources.bucket),
                    format!("{prefix}s3:::{}/*", resources.bucket),
                ],
            },
        ],
    }
}

fn collection_rule(resource_type: &str, collection_name: &str) -> CollectionRule {
    CollectionRule {
        resource_type: resource_type.to_owned(),
        resource: vec![format!("collection/{collection_name}")],
    }
}

mod tests;
