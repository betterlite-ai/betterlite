import json
from typing import TypeAlias, TypedDict, cast

from litellm.responses.litellm_completion_transformation.transformation import (
    TOOL_CALLS_CACHE,
)
from litellm.responses.litellm_completion_transformation.transformation import (
    LiteLLMCompletionResponsesConfig as ResponsesConfig,
)
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = JsonValue


class CacheEntry(TypedDict):
    key: str
    value: Json


class RequestOptional(TypedDict, total=False):
    instructions: str


class Request(RequestOptional):
    input: str | ResponseInputParam


class Input(TypedDict):
    request: Request
    replay_reasoning: bool
    cache: list[CacheEntry]


def run(input_value: Input) -> Json:
    request = input_value["request"]
    params = cast("dict[str, Json]", dict(request))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L487-L532
    for entry in input_value["cache"]:
        TOOL_CALLS_CACHE.set_cache(entry["key"], entry["value"])  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L158
    messages = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L487-L532
        Json,
        ResponsesConfig.transform_responses_api_input_to_messages(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/litellm_completion_transformation/transformation.py#L487-L532
            input=request["input"],
            responses_api_request=params,
            replay_reasoning=input_value["replay_reasoning"],
        ),
    )
    for entry in input_value["cache"]:
        TOOL_CALLS_CACHE.delete_cache(entry["key"])  # type: ignore[no-untyped-call]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/in_memory_cache.py#L268
    wire = TypeAdapter(JsonValue)
    return wire.validate_json(json.dumps(messages))
