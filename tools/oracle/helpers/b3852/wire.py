from typing import Final, Literal, TypeAlias, TypedDict

import litellm
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

Fields: TypeAlias = dict[str, JsonValue]
Params: TypeAlias = dict[str, str]

KIND = "b3852_memory"
CALL_ID = "call-1"
MODEL = "gpt-4o"
START_SECONDS = 1_704_067_200.0
END_SECONDS = START_SECONDS + 1.5
# log_pre_api_call takes messages untyped, so an empty list literal there would be list[Any]
MESSAGES: Final[list[JsonValue]] = []
JSON: Final = TypeAdapter(JsonValue)

Function: TypeAlias = Literal[
    "pre_call",
    "mcp_tool_call",
    "metric_error",
    "close_dedup",
    "cache_eviction",
    "project_headers",
]


class ExporterInput(TypedDict):
    kind: str
    endpoint: str | None
    headers: str | None
    owner: str | None
    use_simple_processor: bool


class Input(TypedDict):
    function: Function
    callback_name: str | None
    anchored: bool
    exporters: list[ExporterInput]
    credentials: Params | None
    auth_metadata: Params | None
    spaces: list[str]
    project_name: str | None
    filter: JsonValue
