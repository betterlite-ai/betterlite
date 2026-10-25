from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.llms.a2a import common_utils

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# LiteLLM types the message as a dict, while the oracle sends every JSON shape on purpose
extract_message = cast(Callable[..., str], common_utils.extract_text_from_a2a_message)
extract_response = cast(Callable[..., str], common_utils.extract_text_from_a2a_response)


def run(input_value: dict[str, Json]) -> str:
    max_depth = input_value["max_depth"]
    limit = {} if max_depth is None else {"max_depth": max_depth}
    if input_value["function"] == "message":
        depth = input_value["depth"]
        start = {} if depth is None else {"depth": depth}
        return extract_message(input_value["message"], **start, **limit)
    return extract_response(input_value["response"], **limit)
