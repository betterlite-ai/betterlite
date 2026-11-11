use betterlite_context::aws_partition::get_aws_partition;
use uuid::Uuid;

use crate::bedrock_ingestion::wire::{
    AossConfiguration, AossFieldMapping, CreateKnowledgeBaseRequest, KnowledgeBaseConfiguration,
    KnowledgeBaseType, StorageConfiguration, StorageType, VectorConfiguration,
};

pub(super) fn create_request(
    region: &str,
    embedding_model: &str,
    kb_name: &str,
    role_arn: &str,
    collection_arn: &str,
) -> CreateKnowledgeBaseRequest {
    let arn_prefix = get_aws_partition(Some(region)).arn_prefix;
    CreateKnowledgeBaseRequest {
        client_token: Uuid::new_v4().to_string(),
        name: kb_name.to_owned(),
        role_arn: role_arn.to_owned(),
        knowledge_base_configuration: KnowledgeBaseConfiguration {
            knowledge_base_type: KnowledgeBaseType::Vector,
            vector: VectorConfiguration {
                embedding_model_arn: format!(
                    "{arn_prefix}bedrock:{region}::foundation-model/{embedding_model}"
                ),
            },
        },
        storage_configuration: StorageConfiguration {
            storage_type: StorageType::OpensearchServerless,
            aoss: AossConfiguration {
                collection_arn: collection_arn.to_owned(),
                field_mapping: AossFieldMapping {
                    metadata_field: "AMAZON_BEDROCK_METADATA".to_owned(),
                    text_field: "AMAZON_BEDROCK_TEXT_CHUNK".to_owned(),
                    vector_field: "bedrock-knowledge-base-default-vector".to_owned(),
                },
                vector_index_name: "bedrock-kb-index".to_owned(),
            },
        },
    }
}
