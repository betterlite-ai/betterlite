#![cfg(test)]

use super::{
    RoleResources, data_access_policy, encryption_policy, index_body, network_policy,
    permissions_policy, trust_policy,
};

const ACCOUNT: &str = "123456789012";
const CALLER: &str = "arn:aws:iam::123456789012:role/kb";

fn resources<'plan>(arn_prefix: &'plan str, region: &'plan str) -> RoleResources<'plan> {
    RoleResources {
        arn_prefix,
        region,
        account_id: ACCOUNT,
        embedding_model: "amazon.titan-embed-text-v2:0",
        collection_arn: "arn:aws:aoss:us-west-2:123456789012:collection/abc",
        bucket: "litellm-kb-0123abcd",
    }
}

#[test]
fn plan_encryption_policy_matches_the_litellm_document() {
    let document = encryption_policy("litellm-kb-0123abcd");
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        r#"{"Rules":[{"ResourceType":"collection","Resource":["collection/litellm-kb-0123abcd"]}],"AWSOwnedKey":true}"#
    );
}

#[test]
fn plan_network_policy_lists_collection_and_dashboard_rules() {
    let document = network_policy("kb");
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        r#"[{"Rules":[{"ResourceType":"collection","Resource":["collection/kb"]},{"ResourceType":"dashboard","Resource":["collection/kb"]}],"AllowFromPublic":true}]"#
    );
}

#[test]
fn plan_data_access_policy_keeps_root_first_and_normalizes_the_caller() {
    let document = data_access_policy(
        "kb",
        "arn:aws:",
        ACCOUNT,
        "arn:aws:sts::123456789012:assumed-role/kb/session",
    );
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        r#"[{"Rules":[{"ResourceType":"index","Resource":["index/kb/*"],"Permission":["aoss:*"]},{"ResourceType":"collection","Resource":["collection/kb"],"Permission":["aoss:*"]}],"Principal":["arn:aws:iam::123456789012:root","arn:aws:iam::123456789012:role/kb"]}]"#
    );
}

#[test]
fn plan_data_access_policy_drops_a_duplicate_root_principal() {
    let document = data_access_policy("kb", "arn:aws:", ACCOUNT, "arn:aws:iam::123456789012:root");
    assert_eq!(
        document[0].principal,
        vec!["arn:aws:iam::123456789012:root".to_owned()]
    );
}

#[test]
fn plan_data_access_policy_uses_the_partition_arn_prefix() {
    let document = data_access_policy("kb", "arn:aws-cn:", ACCOUNT, CALLER);
    assert_eq!(
        document[0].principal,
        vec![
            "arn:aws-cn:iam::123456789012:root".to_owned(),
            CALLER.to_owned(),
        ]
    );
}

#[test]
fn plan_index_body_sets_the_vector_field() {
    assert_eq!(
        serde_json::to_string(&index_body()).unwrap(),
        r#"{"settings":{"index":{"knn":true,"knn.algo_param.ef_search":512}},"mappings":{"properties":{"bedrock-knowledge-base-default-vector":{"type":"knn_vector","dimension":1024,"method":{"engine":"faiss","name":"hnsw","space_type":"l2"}},"AMAZON_BEDROCK_METADATA":{"type":"text","index":false},"AMAZON_BEDROCK_TEXT_CHUNK":{"type":"text"}}}}"#
    );
}

#[test]
fn plan_trust_policy_names_the_bedrock_service_and_account() {
    let document = trust_policy(&resources("arn:aws:", "us-west-2"));
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Principal":{"Service":"bedrock.amazonaws.com"},"Action":"sts:AssumeRole","Condition":{"StringEquals":{"aws:SourceAccount":"123456789012"},"ArnLike":{"aws:SourceArn":"arn:aws:bedrock:us-west-2:123456789012:knowledge-base/*"}}}]}"#
    );
}

#[test]
fn plan_permissions_policy_lists_the_three_statements() {
    let document = permissions_policy(&resources("arn:aws:", "us-west-2"));
    assert_eq!(
        serde_json::to_string(&document).unwrap(),
        r#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":["bedrock:InvokeModel"],"Resource":["arn:aws:bedrock:us-west-2::foundation-model/amazon.titan-embed-text-v2:0"]},{"Effect":"Allow","Action":["aoss:APIAccessAll"],"Resource":["arn:aws:aoss:us-west-2:123456789012:collection/abc"]},{"Effect":"Allow","Action":["s3:GetObject","s3:ListBucket"],"Resource":["arn:aws:s3:::litellm-kb-0123abcd","arn:aws:s3:::litellm-kb-0123abcd/*"]}]}"#
    );
}

#[test]
fn plan_permissions_policy_uses_the_partition_arn_prefix() {
    let document = permissions_policy(&resources("arn:aws-cn:", "cn-north-1"));
    assert_eq!(
        document.statement[0].resource,
        vec![
            "arn:aws-cn:bedrock:cn-north-1::foundation-model/amazon.titan-embed-text-v2:0"
                .to_owned()
        ]
    );
    assert_eq!(
        document.statement[2].resource,
        vec![
            "arn:aws-cn:s3:::litellm-kb-0123abcd".to_owned(),
            "arn:aws-cn:s3:::litellm-kb-0123abcd/*".to_owned(),
        ]
    );
}
