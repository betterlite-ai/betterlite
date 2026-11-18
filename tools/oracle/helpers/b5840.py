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
CONFIG = SagemakerChatConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L42-L44

litellm.suppress_debug_info = True


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = INPUT.validate_python(input_value)
    optional_params: dict[str, JsonValue] = {}
    litellm_params: dict[str, JsonValue] = {}
    if case["function"] == "url":
        optional_params["aws_region_name"] = case["region"]
        if case["base_url"] is not None:
            optional_params["sagemaker_base_url"] = case["base_url"]
        url = CONFIG.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L84-L92
            None,
            None,
            case["model"],
            optional_params,
            litellm_params,
            case["stream"],
        )
        return {"url": url}
    if case["model_id"] is not None:
        optional_params["model_id"] = case["model_id"]
    headers: dict[str, JsonValue] = CONFIG.validate_environment(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L49-L58
        case["headers"],
        "model",
        [],  # type: ignore[misc]  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/sagemaker/chat/transformation.py#L53
        optional_params,
        litellm_params,
    )
    return {"headers": headers}
