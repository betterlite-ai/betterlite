from typing import NotRequired, Protocol, TypedDict, runtime_checkable

import litellm
from litellm.llms.bedrock.chat.converse_transformation import AmazonConverseConfig
from pydantic import BaseModel, ConfigDict
from pydantic import JsonValue as Json


class MetadataPair(TypedDict):
    key: str
    value: Json


class Input(TypedDict):
    model: str
    messages: list[Json]
    model_cost: dict[str, Json]
    optional_params: dict[str, Json]
    litellm_params: dict[str, Json]
    headers: NotRequired[dict[str, Json]]
    global_drop: bool
    modify_params: bool
    neutralize_orphaned_blocks: bool
    request_metadata_fields: list[str] | None
    request_metadata_order: NotRequired[list[MetadataPair]]


@runtime_checkable
class Transformer(Protocol):
    def transform_request(
        self,
        model: str,
        messages: list[Json],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        headers: dict[str, Json],
    ) -> dict[str, Json]: ...


# the message type carries Any, so the config enters through this JSON-facing signature
class Bound(BaseModel):
    model_config = ConfigDict(arbitrary_types_allowed=True)
    transformer: Transformer


def answer(input_value: Input) -> Json:
    config = AmazonConverseConfig()
    holder: dict[str, AmazonConverseConfig] = {"transformer": config}
    transformer = Bound.model_validate(holder).transformer
    messages: list[Json] = list(input_value["messages"])
    optional_params: dict[str, Json] = dict(input_value["optional_params"])
    # The case lists the members as pairs, so their caller order survives YAML and JSON parsing.
    pair_values = input_value.get("request_metadata_order")
    if pair_values is not None:
        optional_params["requestMetadata"] = {
            pair["key"]: pair["value"] for pair in pair_values
        }
    litellm_params: dict[str, Json] = dict(input_value["litellm_params"])
    header_values = input_value.get("headers")
    headers: dict[str, Json] = {} if header_values is None else header_values
    body = transformer.transform_request(
        input_value["model"], messages, optional_params, litellm_params, headers
    )
    result: dict[str, Json] = {
        "body": body,
        "optional_params": optional_params,
        "litellm_params": litellm_params,
    }
    return result


def run(input_value: Input) -> Json:
    litellm.model_cost = input_value["model_cost"]
    # the proxy sets these; the drop and modify branches read them as globals
    litellm.suppress_debug_info = True
    litellm.drop_params = input_value["global_drop"]
    litellm.modify_params = input_value["modify_params"]
    litellm.bedrock_neutralize_orphaned_tool_blocks = input_value["neutralize_orphaned_blocks"]
    litellm.bedrock_request_metadata_fields = input_value["request_metadata_fields"]
    return answer(input_value)
