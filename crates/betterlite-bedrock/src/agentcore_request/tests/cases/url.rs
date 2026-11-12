use crate::agentcore_request::error::AgentCoreRequestError;
use crate::agentcore_request::payload::transform_request;
use crate::agentcore_request::tests::support::{
    generated_uuid, headers, messages, params, request_data,
};
use crate::agentcore_request::url::complete_url;

const MODEL: &str =
    "agentcore/arn:aws:bedrock-agentcore:eu-west-1:123456789012:runtime/agent_runtime-1.2_~x";
const ENCODED: &str =
    "arn%3Aaws%3Abedrock-agentcore%3Aeu-west-1%3A123456789012%3Aruntime%2Fagent_runtime-1.2_~x";
const URL: &str = "https://bedrock-agentcore.eu-west-1.amazonaws.com/runtimes/arn%3Aaws%3Abedrock-agentcore%3Aeu-west-1%3A123456789012%3Aruntime%2Fagent_runtime-1.2_~x/invocations";

pub(super) fn arn_region() {
    assert_default_endpoint();
    assert_configured_endpoint();
    assert_partitions();
    assert_percent_encoding();
}

fn assert_default_endpoint() {
    assert_eq!(complete_url(MODEL, &params("{}")).unwrap(), URL);
    assert_eq!(
        complete_url(MODEL, &params(r#"{"aws_region_name":"us-east-1"}"#)).unwrap(),
        URL,
        "the AWS region override does not move the URL off the ARN region"
    );
}

fn assert_configured_endpoint() {
    let endpoint = params(r#"{"aws_bedrock_runtime_endpoint":"https://vpce.example/prefix/"}"#);
    assert_eq!(
        complete_url(MODEL, &endpoint).unwrap(),
        format!("https://vpce.example/prefix//runtimes/{ENCODED}/invocations"),
        "a trailing slash is not trimmed"
    );
    assert_eq!(
        complete_url(MODEL, &params(r#"{"aws_bedrock_runtime_endpoint":""}"#)).unwrap(),
        URL
    );
    assert_eq!(
        complete_url(MODEL, &params(r#"{"aws_bedrock_runtime_endpoint":null}"#)).unwrap(),
        URL
    );
    assert!(matches!(
        complete_url(MODEL, &params(r#"{"aws_bedrock_runtime_endpoint":7}"#)),
        Err(AgentCoreRequestError::WrongType {
            field: "aws_bedrock_runtime_endpoint"
        })
    ));
}

fn assert_partitions() {
    let china = "agentcore/arn:aws-cn:bedrock-agentcore:cn-north-1:1:runtime/agent";
    assert_eq!(
        complete_url(china, &params("{}")).unwrap(),
        "https://bedrock-agentcore.cn-north-1.amazonaws.com.cn/runtimes/arn%3Aaws-cn%3Abedrock-agentcore%3Acn-north-1%3A1%3Aruntime%2Fagent/invocations"
    );
    let gov = "agentcore/arn:aws-us-gov:bedrock-agentcore:us-gov-west-1:1:runtime/agent";
    assert_eq!(
        complete_url(gov, &params("{}")).unwrap(),
        "https://bedrock-agentcore.us-gov-west-1.amazonaws.com/runtimes/arn%3Aaws-us-gov%3Abedrock-agentcore%3Aus-gov-west-1%3A1%3Aruntime%2Fagent/invocations"
    );
    let unknown = "agentcore/arn:aws:bedrock-agentcore:mars-1:1:runtime/agent";
    assert_eq!(
        complete_url(unknown, &params("{}")).unwrap(),
        "https://bedrock-agentcore.mars-1.amazonaws.com/runtimes/arn%3Aaws%3Abedrock-agentcore%3Amars-1%3A1%3Aruntime%2Fagent/invocations"
    );
}

fn assert_percent_encoding() {
    let punctuation =
        "agentcore/arn:aws:bedrock-agentcore:us-east-1:1:runtime/id with +&=?#% \u{e9}";
    assert_eq!(
        complete_url(punctuation, &params("{}")).unwrap(),
        "https://bedrock-agentcore.us-east-1.amazonaws.com/runtimes/arn%3Aaws%3Abedrock-agentcore%3Aus-east-1%3A1%3Aruntime%2Fid%20with%20%2B%26%3D%3F%23%25%20%C3%A9/invocations"
    );
}

pub(super) fn qualifier_query() {
    assert_qualifier_values();
    assert_qualifier_not_in_payload();
}

fn assert_qualifier_values() {
    assert_eq!(complete_url(MODEL, &params("{}")).unwrap(), URL);
    assert_eq!(
        complete_url(MODEL, &params(r#"{"qualifier":"prod"}"#)).unwrap(),
        format!("{URL}?qualifier=prod")
    );
    assert_eq!(
        complete_url(MODEL, &params(r#"{"qualifier":""}"#)).unwrap(),
        format!("{URL}?qualifier=")
    );
    assert_eq!(
        complete_url(MODEL, &params(r#"{"qualifier":null}"#)).unwrap(),
        format!("{URL}?qualifier=None")
    );
    assert_eq!(
        complete_url(MODEL, &params(r#"{"qualifier":"a b&c=d?e#f"}"#)).unwrap(),
        format!("{URL}?qualifier=a b&c=d?e#f"),
        "query characters reach the URL as sent"
    );
    let non_ascii = params(&format!(r#"{{"qualifier":"{}"}}"#, '\u{e9}'));
    assert_eq!(
        complete_url(MODEL, &non_ascii).unwrap(),
        format!("{URL}?qualifier={}", '\u{e9}')
    );
    assert!(matches!(
        complete_url(MODEL, &params(r#"{"qualifier":5}"#)),
        Err(AgentCoreRequestError::WrongType { field: "qualifier" })
    ));
}

fn assert_qualifier_not_in_payload() {
    let optional = params(
        r#"{"qualifier":"prod","forward_multimodal_content":true,"temperature":0.5,"stream":true}"#,
    );
    let before = optional.clone();
    let mut request_headers = headers(&[]);
    let payload = transform_request(
        &messages(r#"[{"role":"user","content":"hi"}]"#),
        &optional,
        &params("{}"),
        &mut request_headers,
        generated_uuid(),
    )
    .unwrap();
    assert_eq!(payload, request_data(r#"{"prompt":"hi"}"#));
    assert_eq!(
        optional, before,
        "mapping leaves the optional map unchanged"
    );
}
