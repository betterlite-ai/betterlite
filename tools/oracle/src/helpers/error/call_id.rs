pub(crate) struct Op;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub enabled: bool,
    pub call_id: Option<String>,
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let call_id = if input.enabled {
            input.call_id.as_deref().filter(|id| !id.is_empty())
        } else {
            None
        };
        Ok(serde_json::json!({"result": call_id}))
    }
}
