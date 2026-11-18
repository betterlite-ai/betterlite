from typing import Literal, TypedDict

import litellm
from litellm.llms.sagemaker.chat.transformation import SagemakerChatConfig
from pydantic import JsonValue, TypeAdapter


class UrlInput(TypedDict):
    function: Literal["url"]
    model: str
    region: str
    base_url: str | None
    stream: bool


class ComponentInput(TypedDict):
    function: Literal["component_header"]
    model_id: str | None
    headers: dict[str, str]


Input = UrlInput | ComponentInput
INPUT = TypeAdapter(Input)
CONFIG = SagemakerChatConfig()

litellm.suppress_debug_info = True


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = INPUT.validate_python(input_value)
    if case["function"] == "url":
        optional_params: dict[str, JsonValue] = {"aws_region_name": case["region"]}
        if case["base_url"] is not None:
            optional_params["sagemaker_base_url"] = case["base_url"]
        url = CONFIG.get_complete_url(
            None,
            None,
            case["model"],
            optional_params,
            {},
            case["stream"],
        )
        return {"url": url, "region": case["region"]}
    optional_params = {}
    if case["model_id"] is not None:
        optional_params["model_id"] = case["model_id"]
    return {
        "headers": CONFIG.validate_environment(
            case["headers"], "model", [], optional_params, {}
        )
    }
