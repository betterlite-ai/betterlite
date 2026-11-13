from collections.abc import Callable, Mapping, Sequence
from typing import TypeAlias, cast

import litellm
from litellm.integrations.otel.model import payloads

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

OutputChoices: TypeAlias = Callable[[Mapping[str, object]], Sequence[Mapping[str, object]]]
FinishReasons: TypeAlias = Callable[[Sequence[Mapping[str, object]]], Sequence[str]]


def run(input_value: dict[str, Json]) -> Json:
    return {"result": dispatch(input_value)}


def dispatch(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "output_choices":
        response = input_value.get("response")
        if not isinstance(response, dict):
            raise ValueError("response must be an object")
        name = "_output_choices"
        operation = cast(OutputChoices, getattr(payloads, name))
        return cast(Json, [dict(choice) for choice in operation(response)])
    if function == "finish_reasons":
        choices = input_value.get("choices")
        if not isinstance(choices, list):
            raise ValueError("choices must be a list")
        name = "_finish_reasons"
        operation = cast(FinishReasons, getattr(payloads, name))
        return cast(
            Json,
            list(
                operation(
                    [cast(Mapping[str, object], choice) for choice in choices]
                )
            ),
        )
    raise ValueError(f"unknown LiteLLM helper function {function} ({litellm.__name__})")
