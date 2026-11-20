import asyncio
from typing import TypedDict, cast

from litellm.proxy.common_request_processing import (
    _extract_error_from_sse_chunk,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L772
    _parse_event_data_for_error,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L737
)
from pydantic import JsonValue


class Input(TypedDict):
    text: str


def run(input_value: Input) -> JsonValue:
    text = input_value["text"]
    code = asyncio.run(_parse_event_data_for_error(text))
    # LiteLLM returns a bare dict here, and pydantic's recursive alias types it
    error = cast("dict[str, JsonValue]", _extract_error_from_sse_chunk(text))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/common_request_processing.py#L772
    answer: dict[str, JsonValue] = {"code": code, "error": error}
    return answer
