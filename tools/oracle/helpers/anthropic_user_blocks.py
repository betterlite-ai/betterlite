from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import (
    anthropic_messages_pt,
    get_system_prompt,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class _DataUrlLookup:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


def _messages(input_value: dict[str, Json]) -> list[AllMessageValues]:
    return cast("list[AllMessageValues]", input_value["messages"])


def _user_blocks(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    with patch.object(factory, "convert_url_to_base64", _DataUrlLookup(mapping)):
        return cast(
            Json,
            anthropic_messages_pt(
                messages=_messages(input_value),
                model=cast(str, input_value["model"]),
                llm_provider=cast(str, input_value["llm_provider"]),
            ),
        )


def _system_prompt(input_value: dict[str, Json]) -> Json:
    prompt, remaining = cast(
        "tuple[str, list[Json]]", get_system_prompt(_messages(input_value))
    )
    return cast(Json, [prompt, remaining])


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "user_blocks":
        return _user_blocks(input_value)
    if function == "system_prompt":
        return _system_prompt(input_value)
    raise ValueError(f"unknown anthropic user blocks function: {function!r}")
