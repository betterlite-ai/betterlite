from typing import TypeAlias, cast

from litellm.llms.anthropic.common_utils import (
    flatten_unencrypted_web_search_results_in_anthropic_messages,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    messages = input_value.get("messages")
    if not isinstance(messages, list):
        raise ValueError("messages must be a list")
    flattened = flatten_unencrypted_web_search_results_in_anthropic_messages(messages)
    return cast(Json, flattened)
