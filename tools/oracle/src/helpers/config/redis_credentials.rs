use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    GcpTokenCaching { service_account: String },
    ElasticacheSigGeneration { cache_name: String, #[serde(default)] serverless: bool },
    AzureCredentialSelection {
        #[serde(default)] client_id: Option<String>,
        #[serde(default)] tenant_id: Option<String>,
        #[serde(default)] client_secret: Option<String>,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::GcpTokenCaching { service_account } => json!({
                "provider": "gcp",
                "service_account_configured": !service_account.is_empty(),
                "token_ttl_seconds": 3300,
            }),
            Input::ElasticacheSigGeneration { cache_name, serverless } => json!({
                "provider": "aws",
                "cache_name": cache_name.to_ascii_lowercase(),
                "token_ttl_seconds": 900,
                "query": if *serverless { "Action=connect&ResourceType=ServerlessCache" } else { "Action=connect" },
            }),
            Input::AzureCredentialSelection { client_id, tenant_id, client_secret } => json!({
                "provider": "azure",
                "credential": if client_id.is_some() && tenant_id.is_some() && client_secret.is_some() { "client_secret" } else if client_id.is_some() { "managed_identity" } else { "default" },
            }),
        };
        Ok(result)
    }
}
