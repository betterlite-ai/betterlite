from typing import TypeAlias, cast
from unittest.mock import patch

import litellm
from litellm.litellm_core_utils.prompt_templates import factory
from litellm.litellm_core_utils.prompt_templates.factory import anthropic_messages_pt
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

DEFAULT_ID = "generated-id"


class _DataUrlLookup:
    def __init__(self, mapping: dict[str, str]) -> None:
        self.mapping = mapping

    def __call__(self, url: str) -> str:
        return self.mapping[url]


class _FixedUuid:
    def __init__(self, value: str) -> None:
        self.value = value

    def uuid4(self) -> str:
        return self.value


def _messages(input_value: dict[str, Json]) -> list[AllMessageValues]:
    return cast("list[AllMessageValues]", input_value["messages"])


def run(input_value: dict[str, Json]) -> Json:
    litellm.modify_params = cast(bool, input_value.get("modify_params", False))
    mapping = cast("dict[str, str]", input_value.get("data_urls", {}))
    fixed_id = cast(str, input_value.get("id", DEFAULT_ID))
    messages = _messages(input_value)
    with (
        patch.object(factory, "convert_url_to_base64", _DataUrlLookup(mapping)),
        patch.object(factory, "uuid", _FixedUuid(fixed_id)),
    ):
        return cast(
            Json,
            anthropic_messages_pt(
                messages=messages,
                model=cast(str, input_value["model"]),
                llm_provider=cast(str, input_value["llm_provider"]),
            ),
        )
