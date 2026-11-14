from collections.abc import Sequence
from typing import Literal

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict

from betterlite.types.integrations.custom_logger import StandardCustomLoggerInitParams


class ToolCall(TypedDict, total=False):
    name: ReadOnly[str]
    arguments: ReadOnly[dict[str, JsonValue] | str]
    tool_id: ReadOnly[str]
    type: ReadOnly[str]


class ToolResult(TypedDict, total=False):
    name: ReadOnly[str]
    result: ReadOnly[str]
    tool_id: ReadOnly[str]
    type: ReadOnly[str]


class ToolDefinition(TypedDict, total=False):
    name: ReadOnly[str]
    description: ReadOnly[str]
    schema: ReadOnly[dict[str, JsonValue]]


class Message(TypedDict, total=False):
    content: ReadOnly[str]
    role: ReadOnly[str]
    reasoning_content: ReadOnly[str]
    tool_calls: ReadOnly[Sequence[ToolCall]]
    tool_results: ReadOnly[Sequence[ToolResult]]


class InputMeta(TypedDict):
    messages: Sequence[Message | dict[str, JsonValue]]


class OutputMeta(TypedDict):
    messages: Sequence[JsonValue]


class DDLLMObsError(TypedDict, total=False):
    message: str
    stack: str | None
    type: str | None


class Meta(TypedDict, total=False):
    kind: Literal["llm", "tool", "task", "embedding", "retrieval"]
    input: InputMeta
    output: OutputMeta
    metadata: dict[str, JsonValue]
    error: DDLLMObsError | None
    tool_definitions: ReadOnly[Sequence[ToolDefinition]]


class LLMMetrics(TypedDict, total=False):
    input_tokens: float
    output_tokens: float
    total_tokens: float
    time_to_first_token: float
    time_per_output_token: float
    total_cost: float
    cache_read_input_tokens: ReadOnly[float]
    cache_write_input_tokens: ReadOnly[float]
    non_cached_input_tokens: ReadOnly[float]
    reasoning_output_tokens: ReadOnly[float]
    tool_output_tokens: ReadOnly[float]


class LLMObsPayload(TypedDict, total=False):
    parent_id: str
    trace_id: str
    apm_id: str
    span_id: str
    name: str
    meta: Meta
    start_ns: int
    duration: int
    metrics: LLMMetrics
    tags: list[JsonValue]
    status: Literal["ok", "error"]


class DDSpanAttributes(TypedDict):
    ml_app: str
    tags: list[str]
    spans: list[LLMObsPayload]


class DDIntakePayload(TypedDict):
    type: str
    attributes: DDSpanAttributes


class DatadogLLMObsInitParams(StandardCustomLoggerInitParams):
    pass


class DDLLMObsLatencyMetrics(TypedDict, total=False):
    time_to_first_token_ms: float
    litellm_overhead_time_ms: float
    guardrail_overhead_time_ms: float


class DDLLMObsSpendMetrics(TypedDict, total=False):
    response_cost: float
    user_api_key_spend: float
    user_api_key_max_budget: float
    user_api_key_budget_reset_at: str
