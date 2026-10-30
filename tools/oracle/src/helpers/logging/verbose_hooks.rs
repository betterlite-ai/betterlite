use betterlite_logging::verbose_hooks::{HookInput, VerboseHooks};

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = HookInput;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        serde_json::to_value(VerboseHooks.run(input.clone())).map_err(|error| {
            Box::new(crate::helpers::HelperError {
                class: "SerializationError".to_owned(),
                status: None,
                error_type: None,
                param: None,
                code: None,
                message: error.to_string(),
            })
        })
    }
}

#[path = "verbose_hooks/tests.rs"]
mod tests;
