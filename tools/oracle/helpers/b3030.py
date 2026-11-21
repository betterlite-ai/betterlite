from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.common_utils.proxy_rate_limit_error import (
    ProxyRateLimitError,
    map_v3_rate_limit_type,
)
from litellm.proxy.hooks.rate_limiter_utils import convert_priority_to_percent
from litellm.types.router import ModelGroupInfo
from litellm.types.utils import PriorityReservationDict
from pydantic import JsonValue, TypeAdapter


class MapTypeInput(TypedDict):
    function: Literal["map_type"]
    value: str | None


class CoerceMessageInput(TypedDict):
    function: Literal["coerce_message"]
    detail: JsonValue


class ModelInfoInput(TypedDict):
    rpm: int | None
    tpm: int | None


class PriorityPercentInput(TypedDict):
    function: Literal["priority_percent"]
    value: float | PriorityReservationDict
    model_info: ModelInfoInput | None


class ModelInfoPayload(TypedDict):
    model_group: str
    providers: list[str]
    rpm: int | None
    tpm: int | None


Input: TypeAlias = MapTypeInput | CoerceMessageInput | PriorityPercentInput


def run(input_value: Input) -> dict[str, JsonValue]:
    if input_value["function"] == "map_type":
        return {"rate_limit_type": map_v3_rate_limit_type(input_value["value"])}
    if input_value["function"] == "coerce_message":
        error = ProxyRateLimitError(detail=input_value["detail"])
        # RateLimitError prefixes .message; the coerced detail text is what both sides compare.
        return {"message": error.message.removeprefix("litellm.RateLimitError: ")}
    info = input_value["model_info"]
    model_info = (
        None
        if info is None
        else TypeAdapter(ModelGroupInfo).validate_python(
            ModelInfoPayload(
                model_group="oracle",
                providers=[],
                rpm=info["rpm"],
                tpm=info["tpm"],
            )
        )
    )
    return {"percent": convert_priority_to_percent(input_value["value"], model_info)}
