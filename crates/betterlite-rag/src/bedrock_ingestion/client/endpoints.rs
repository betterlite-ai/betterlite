use betterlite_context::aws_partition::get_aws_partition;
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};

const US_EAST_1: &str = "us-east-1";
const CREATE_BUCKET_NAMESPACE: &str = "http://s3.amazonaws.com/doc/2006-03-01/";
pub(super) const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}')
    .add(b'/')
    .add(b'\\');

#[must_use]
pub fn bedrock_agent_endpoint(region: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    format!("https://bedrock-agent.{region}.{suffix}")
}

#[must_use]
pub fn knowledge_bases_url(region: &str) -> String {
    let base = bedrock_agent_endpoint(region);
    format!("{base}/knowledgebases/")
}

#[must_use]
pub fn knowledge_base_url(region: &str, knowledge_base_id: &str) -> String {
    let base = bedrock_agent_endpoint(region);
    let id = utf8_percent_encode(knowledge_base_id, PATH_SEGMENT);
    format!("{base}/knowledgebases/{id}")
}

#[must_use]
pub fn data_sources_url(region: &str, knowledge_base_id: &str) -> String {
    let base = bedrock_agent_endpoint(region);
    let id = utf8_percent_encode(knowledge_base_id, PATH_SEGMENT);
    format!("{base}/knowledgebases/{id}/datasources/")
}

#[must_use]
pub fn data_source_url(region: &str, knowledge_base_id: &str, data_source_id: &str) -> String {
    let base = bedrock_agent_endpoint(region);
    let knowledge_base = utf8_percent_encode(knowledge_base_id, PATH_SEGMENT);
    let data_source = utf8_percent_encode(data_source_id, PATH_SEGMENT);
    format!("{base}/knowledgebases/{knowledge_base}/datasources/{data_source}")
}

// boto3's virtual-hosted rule, with the path form for a bucket a TLS certificate cannot name
#[must_use]
pub fn s3_bucket_host(bucket: &str, region: &str) -> String {
    let suffix = get_aws_partition(Some(region)).dns_suffix;
    let path_style = bucket.contains('.') || bucket.bytes().any(|byte| byte.is_ascii_uppercase());
    if path_style {
        return format!("https://s3.{region}.{suffix}/{bucket}");
    }
    if region == US_EAST_1 {
        return format!("https://{bucket}.s3.{suffix}");
    }
    format!("https://{bucket}.s3.{region}.{suffix}")
}

#[must_use]
pub fn create_bucket_body(region: &str) -> Option<String> {
    (region != US_EAST_1).then(|| {
        format!(
            "<CreateBucketConfiguration xmlns=\"{CREATE_BUCKET_NAMESPACE}\"><LocationConstraint>{region}</LocationConstraint></CreateBucketConfiguration>"
        )
    })
}
