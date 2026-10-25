#[path = "../../../../../crates/betterlite-guardrails/src/cisco_conversation.rs"]
mod implementation;

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        crate::helpers::HelperResult::from(
            implementation::run(input)
                .map_err(|message| Box::new(crate::helpers::HelperError {
                    class: "ValueError".to_owned(),
                    status: None,
                    error_type: None,
                    param: None,
                    code: None,
                    message,
                })),
        )
    }
}
