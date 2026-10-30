use betterlite_db::team_rows::{LiteLLM_TeamMembership, LiteLLM_TeamTable, Member, MemberBase};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let operation = input
            .get("operation")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| error("Input", "operation is required"))?;
        let row = input
            .get("row")
            .cloned()
            .ok_or_else(|| error("Input", "row is required"))?;
        let output = match operation {
            "member" => serde_json::to_value(
                serde_json::from_value::<Member>(row)
                    .map_err(|source| error("ValidationError", &source.to_string()))?,
            ),
            "member_base" => {
                MemberBase::validate(&row)
                    .map_err(|source| error("ValidationError", &source.to_string()))?;
                serde_json::to_value(row)
            }
            "team" => serde_json::to_value(
                serde_json::from_value::<LiteLLM_TeamTable>(row)
                    .map_err(|source| error("ValidationError", &source.to_string()))?,
            ),
            "membership" => serde_json::to_value(
                serde_json::from_value::<LiteLLM_TeamMembership>(row)
                    .map_err(|source| error("ValidationError", &source.to_string()))?,
            ),
            _ => return Err(error("Input", "unsupported operation")),
        };
        output.map_err(|source| error("Output", &source.to_string()))
    }
}

fn error(class: &str, message: &str) -> Box<crate::helpers::HelperError> {
    let error = crate::helpers::HelperError {
        class: class.to_owned(),
        status: None,
        error_type: None,
        param: None,
        code: None,
        message: message.to_owned(),
    };
    Box::new(error)
}
