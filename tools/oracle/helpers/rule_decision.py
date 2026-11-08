from typing import Final, TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.rules import Rules
from litellm.main import mock_completion  # pyright: ignore[reportUnknownVariableType]
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

MESSAGES: Final[list[dict[str, str]]] = [{"role": "user", "content": "hi"}]


class _FixedRule:
    def __init__(self, decision: Json) -> None:
        self._decision = decision

    def __call__(self, _input: str) -> Json:
        return self._decision


def _string(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def _list(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return cast(list[Json], value)


def _rule_input(input_value: dict[str, Json], model: str) -> str | None:
    mock_response = input_value.get("mock_response")
    if mock_response is not None:
        result = cast(
            ModelResponse,
            mock_completion(
                model, MESSAGES, mock_response=_string(mock_response, "mock_response")
            ),
        )
        return result.choices[0].message.content
    explicit = input_value.get("input")
    return None if explicit is None else _string(explicit, "input")


def run(input_value: dict[str, Json]) -> Json:
    model = _string(input_value.get("model"), "model")
    phase = _string(input_value.get("phase"), "phase")
    entries = _list(input_value.get("rules"), "rules")
    configured: list[object] = [
        None if entry is None else _FixedRule(entry) for entry in entries
    ]
    text = _rule_input(input_value, model)
    if phase == "pre":
        if text is None:
            raise ValueError("a pre-call rule needs an input")
        with patch.object(litellm, "pre_call_rules", configured):
            Rules().pre_call_rules(input=text, model=model)
    elif phase == "post":
        with patch.object(litellm, "post_call_rules", configured):
            Rules().post_call_rules(input=text, model=model)
    else:
        raise ValueError(f"unknown phase {phase}")
    return True
