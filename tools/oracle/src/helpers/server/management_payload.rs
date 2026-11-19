use betterlite_db::managed_resource_rows::mcp::{McpEnvVar, McpServerTable};
use betterlite_db::mcp_stored_contracts::MCPTransport;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_oracle_helper_api::error::HelperError;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_server::management_payload::redaction::{
    redact_env_var_values, redact_model_env_vars, redact_record_env_vars,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "function", rename_all = "snake_case")]
pub(crate) enum Input {
    RedactEnvVars {
        response: JsonObject,
    },
    RedactMapping {
        record: JsonObject,
    },
    RedactModel {
        server_id: String,
        transport: MCPTransport,
        #[serde(default)]
        env_vars: Option<Vec<McpEnvVar>>,
    },
    RedactOther {
        record: Json,
    },
}

pub(crate) struct Op;

fn helper_error(message: String) -> Box<HelperError> {
    Box::new(HelperError {
        class: "InvalidInput".to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message,
    })
}

fn model_record(
    server_id: &str,
    transport: MCPTransport,
    env_vars: Option<&[McpEnvVar]>,
) -> Result<McpServerTable, Box<HelperError>> {
    let fields = json!({
        "server_id": server_id,
        "transport": transport,
        "env_vars": env_vars,
    });
    serde_json::from_value(fields).map_err(|error| helper_error(error.to_string()))
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::RedactEnvVars { response } => {
                let result = Json::Object(response.clone());
                let mut redacted = response.clone();
                redact_env_var_values(&mut redacted);
                Ok(json!({"response": redacted, "result": result}))
            }
            Input::RedactMapping { record } => {
                let record = Json::Object(record.clone());
                Ok(json!({"record": redact_record_env_vars(&record)}))
            }
            Input::RedactModel {
                server_id,
                transport,
                env_vars,
            } => {
                let record = model_record(server_id, *transport, env_vars.as_deref())?;
                let redacted = redact_model_env_vars(&record);
                Ok(json!({"env_vars": redacted.env_vars}))
            }
            Input::RedactOther { record } => Ok(json!({"record": redact_record_env_vars(record)})),
        }
    }
}
