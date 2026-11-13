from typing import TypedDict

from litellm.litellm_core_utils.aws_partition import (
    contains_aws_arn,
    contains_bedrock_arn,
    get_aws_arn_prefix,
    get_aws_partition,
    is_bedrock_arn,
)


class Input(TypedDict):
    region: str | None
    value: str


def run(data: Input) -> dict[str, object]:
    partition = get_aws_partition(data.get("region"))
    value = data.get("value", "")
    return {
        "partition": partition.partition,
        "dns_suffix": partition.dns_suffix,
        "arn_prefix": get_aws_arn_prefix(data.get("region")),
        "contains_bedrock_arn": contains_bedrock_arn(value),
        "is_bedrock_arn": is_bedrock_arn(value),
        "contains_aws_arn": contains_aws_arn(value),
    }
