use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct SessionPolicy {
    pub version: &'static str,
    pub statement: Vec<Statement>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct Statement {
    pub sid: &'static str,
    pub effect: &'static str,
    pub action: &'static [&'static str],
    pub resource: &'static str,
    pub condition: Condition,
}

#[derive(Debug, Serialize)]
pub struct Condition {
    #[serde(rename = "Bool")]
    pub secure: Secure,
}

#[derive(Debug, Serialize)]
pub struct Secure {
    #[serde(rename = "aws:SecureTransport")]
    pub enabled: &'static str,
}

const ACTIONS: [(&str, &[&str]); 4] = [
    (
        "BedrockLiteLLM",
        &[
            "bedrock:InvokeModel",
            "bedrock:InvokeModelWithResponseStream",
            "bedrock:CountTokens",
            "bedrock:Rerank",
            "bedrock:Retrieve",
            "bedrock:ListKnowledgeBases",
            "bedrock:InvokeAgent",
            "bedrock:ApplyGuardrail",
            "bedrock:GetGuardrail",
            "bedrock:ListGuardrails",
        ],
    ),
    (
        "BedrockAgentCoreLiteLLM",
        &[
            "bedrock-agentcore:InvokeAgentRuntime",
            "bedrock-agentcore:InvokeAgentRuntimeForUser",
            "bedrock-agentcore:InvokeGateway",
        ],
    ),
    (
        "ClaudePlatformLiteLLM",
        &[
            "aws-external-anthropic:CreateInference",
            "aws-external-anthropic:CreateBatchInference",
            "aws-external-anthropic:CancelBatchInference",
            "aws-external-anthropic:DeleteBatchInference",
            "aws-external-anthropic:CountTokens",
            "aws-external-anthropic:Get*",
            "aws-external-anthropic:List*",
        ],
    ),
    ("BedrockMantleLiteLLM", &["bedrock-mantle:CreateInference"]),
];

#[must_use]
pub fn web_identity_policy() -> SessionPolicy {
    let statement = ACTIONS
        .iter()
        .map(|(sid, action)| Statement {
            sid,
            action,
            effect: "Allow",
            resource: "*",
            condition: Condition {
                secure: Secure { enabled: "true" },
            },
        })
        .collect();
    SessionPolicy {
        version: "2012-10-17",
        statement,
    }
}
