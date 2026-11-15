use betterlite_db::budget_permission_rows::JsonData;
use betterlite_db::identity_rows::key::LiteLLMVerificationToken;
use betterlite_db::identity_rows::user::LiteLLMUserTable;
use betterlite_db::tool_records::decode::decode_tool_record;
use betterlite_db::tool_records::owners::{attach_owners, owners_from_rows};
use betterlite_db::tool_records::records::LiteLLMToolTableRow;

pub(crate) struct Op;

#[derive(serde::Deserialize)]
pub(crate) struct Input {
    rows: Vec<JsonData>,
    #[serde(default)]
    tokens: Vec<LiteLLMVerificationToken>,
    #[serde(default)]
    users: Vec<LiteLLMUserTable>,
}

#[derive(serde::Serialize)]
struct Output {
    tools: Vec<LiteLLMToolTableRow>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let mut tools = Vec::with_capacity(input.rows.len());
        for row in &input.rows {
            let Ok(text) = serde_json::to_string(row) else {
                tools.clear();
                break;
            };
            let Ok(tool) = decode_tool_record(text.as_bytes()) else {
                tools.clear();
                break;
            };
            tools.push(tool);
        }
        let owners = owners_from_rows(&input.tokens, &input.users);
        attach_owners(&mut tools, &owners);
        serde_json::to_value(Output { tools }).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "ValueError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}
