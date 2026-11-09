from collections.abc import Mapping
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.datadog import datadog_llm_obs

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_LOGGER = object.__new__(datadog_llm_obs.DataDogLLMObsLogger)  # pyright: ignore[reportPrivateUsage]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "messages":
        messages = input_value.get("messages")
        return plain(datadog_llm_obs._to_dd_messages(messages))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if function == "tool_arguments":
        return [
            plain(datadog_llm_obs._to_dd_arguments(piece))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            for piece in argument_pieces(input_value.get("arguments"))
        ]
    if function == "tool_definitions":
        model_parameters = input_value.get("model_parameters")
        return plain(datadog_llm_obs._to_dd_tool_definitions(model_parameters))  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    if function == "response_messages":
        payload = cast("StandardLoggingPayload", {"response": input_value.get("response")})
        call_type = optional_string(input_value.get("call_type"))
        return plain(_LOGGER._get_response_messages(payload, call_type))  # pyright: ignore[reportPrivateUsage]
    raise ValueError(f"unknown {litellm.__name__} helper function {function}")


def argument_pieces(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError("arguments must be a list")
    pieces: list[Json] = []
    for piece in value:
        if isinstance(piece, dict) and "times" in piece:
            text = own_string(piece.get("text", ""), "text")
            prefix = own_string(piece.get("prefix", ""), "prefix")
            suffix = own_string(piece.get("suffix", ""), "suffix")
            times = piece.get("times")
            if isinstance(times, bool) or not isinstance(times, int):
                raise ValueError("a repeated argument needs an integer times")
            pieces.append(cast(Json, prefix + text * times + suffix))
        else:
            pieces.append(piece)
    return pieces


def own_string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json) -> str | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise ValueError("call_type must be a string or null")
    return value


def plain(value: object) -> Json:
    if isinstance(value, Mapping):
        fields = cast("Mapping[object, object]", value)
        return {str(key): plain(item) for key, item in fields.items()}
    if isinstance(value, (list, tuple)):
        return [plain(item) for item in cast("tuple[object, ...]", value)]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return cast(Json, value)
    raise ValueError(f"cannot represent a {type(value).__name__} in a helper answer")
