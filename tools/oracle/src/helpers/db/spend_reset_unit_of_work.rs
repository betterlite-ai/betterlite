pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = serde_json::Value;

    fn run(_input: &Self::Input) -> crate::helpers::HelperResult {
        Self::not_built("spend_reset_unit_of_work")
    }
}
