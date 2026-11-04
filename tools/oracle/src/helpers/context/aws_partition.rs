use betterlite_context::aws_partition::{
    contains_aws_arn, contains_bedrock_arn, get_aws_partition, is_bedrock_arn,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    region: Option<String>,
    value: Option<String>,
}
pub(crate) struct Op;
impl crate::helpers::Helper for Op {
    type Input = Input;
    fn run(input: &Input) -> crate::helpers::HelperResult {
        let partition = get_aws_partition(input.region.as_deref());
        let value = input.value.as_deref().unwrap_or_default();
        Ok(
            json!({"partition":partition.partition,"dns_suffix":partition.dns_suffix,"arn_prefix":format!("arn:{}:", partition.partition),"contains_bedrock_arn":contains_bedrock_arn(value),"is_bedrock_arn":is_bedrock_arn(value),"contains_aws_arn":contains_aws_arn(value)}),
        )
    }
}
