use std::collections::BTreeMap;

use betterlite_rag::bedrock_ingestion::settings::VectorStoreConfig;
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub(super) enum Input {
    Principal {
        cases: Vec<PrincipalCase>,
    },
    Detect {
        vector: VectorStoreConfig,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        data_sources: Value,
        data_source: Value,
    },
    Bucket {
        cases: Vec<BucketCase>,
    },
    CreateKnowledgeBase {
        vector: VectorStoreConfig,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        kb_name: String,
        knowledge_base_id: String,
        role_arn: String,
        collection_arn: String,
    },
    CreateDataSource {
        vector: VectorStoreConfig,
        #[serde(default)]
        environment: BTreeMap<String, String>,
        kb_name: String,
        data_source_id: String,
    },
}

#[derive(Debug, Deserialize)]
pub(super) struct PrincipalCase {
    pub(super) caller_arn: String,
    pub(super) account_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct BucketCase {
    pub(super) vector: VectorStoreConfig,
    #[serde(default)]
    pub(super) environment: BTreeMap<String, String>,
    pub(super) unique_id: String,
}
