from collections.abc import Callable
from typing import Final, Protocol, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.image_edit.amazon_nova_canvas_image_edit_transformation import (
    BedrockAmazonNovaCanvasImageEditConfig as NovaConfig,
)
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ImageResponse
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"


class ResponseCall(TypedDict):
    model: str
    status: int
    body: str


class Input(TypedDict):
    model_cost: dict[str, JsonValue]
    calls: list[ResponseCall]


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


HIDDEN: Final = TypeAdapter(Hidden)


class Sink(Logging):
    def __init__(self) -> None:
        pass


class EditConfig(Protocol):
    @property
    def transform_image_edit_response(
        self,
    ) -> Callable[[str, httpx.Response, Logging], ImageResponse]: ...


def image_entries(response: ImageResponse) -> list[JsonValue]:
    images = response.data
    if images is None:
        return []
    return [
        {"b64_json": image.b64_json} for image in images if image.b64_json is not None
    ]


def response_answer(call: ResponseCall, config: EditConfig) -> JsonValue:
    upstream = httpx.Response(call["status"], headers=None, content=call["body"].encode())
    try:
        response = config.transform_image_edit_response(call["model"], upstream, Sink())
    except BedrockError as error:
        return {"ok": False, "status": error.status_code}
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return {
        "ok": True,
        "images": image_entries(response),
        "cost": hidden["additional_headers"].get(COST_HEADER),
    }


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    config: EditConfig = NovaConfig()
    return [response_answer(call, config) for call in input_value["calls"]]
