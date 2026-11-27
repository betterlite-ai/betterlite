from typing import Final, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.utils import ImageResponse
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter
from typing_extensions import override

COST_HEADER: Final = "llm_provider-x-litellm-response-cost"
# what httpx adds itself and what the signature covers never enter the shared answer
OMITTED_HEADERS: Final = frozenset(
    {
        "accept",
        "accept-encoding",
        "authorization",
        "connection",
        "host",
        "user-agent",
        "x-amz-date",
        "x-amz-security-token",
    }
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class Hidden(TypedDict):
    additional_headers: dict[str, JsonValue]


HIDDEN: Final[TypeAdapter[Hidden]] = TypeAdapter(Hidden)


class AdditionalArgs(BaseModel):
    model_config = ConfigDict(extra="allow")

    api_base: str | None = None
    complete_input_dict: dict[str, JsonValue] | None = None


ARGS: Final[TypeAdapter[AdditionalArgs]] = TypeAdapter(AdditionalArgs)


class Recorder(Logging):
    def __init__(self) -> None:
        self.pre: dict[str, JsonValue] | None = None
        self.post: dict[str, JsonValue] | None = None

    @override
    def pre_call(self, *_args: JsonValue, **values: JsonValue) -> None:
        additional = ARGS.validate_python(values.get("additional_args"))
        self.pre = {
            "input": values.get("input"),
            "api_base": additional.api_base,
            "complete_input_dict": additional.complete_input_dict,
        }

    @override
    def post_call(self, *_args: JsonValue, **values: JsonValue) -> None:
        additional = ARGS.validate_python(values.get("additional_args"))
        record: dict[str, JsonValue] = {
            "input": values.get("input"),
            "original_response": values.get("original_response"),
        }
        record["complete_input_dict"] = additional.complete_input_dict
        self.post = record


def shown_headers(request: httpx.Request) -> dict[str, str]:
    kept = (
        (name, value)
        for name, value in request.headers.items()
        if name not in OMITTED_HEADERS
    )
    return dict(kept)


def image_entries(response: ImageResponse) -> list[JsonValue]:
    images = response.data
    if images is None:
        return []
    return [
        {"b64_json": image.b64_json} for image in images if image.b64_json is not None
    ]


def cost_of(response: ImageResponse) -> JsonValue:
    hidden = HIDDEN.validate_python(get_hidden_params_dict(response))
    return hidden["additional_headers"].get(COST_HEADER)
