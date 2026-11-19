import asyncio
from datetime import UTC, datetime
from typing import Final, Literal, Required, TypedDict

from litellm.integrations.compression_interception.handler import CompressionInterceptionLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.utils import CallTypes
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MESSAGES = TypeAdapter(list[dict[str, object]])
SEED_MODEL: Final = "claude-3"
SEED_MESSAGES: Final[list[dict[str, JsonValue]]] = [
    {"role": "assistant", "content": "alpha report"},
    {"role": "user", "content": "and another question about beta"},
    {"role": "user", "content": "what did the report say"},
]
SEED_TRIGGER: Final = 1
SEED_TARGET: Final = 1


class Seed(TypedDict):
    call_id: str
    cache: dict[str, str]


class LoggingSpec(TypedDict):
    call_id: str
    agentic_model: str | None


class Input(TypedDict, total=False):
    function: Required[Literal["gate", "plan"]]
    model: Required[str]
    enabled: bool
    response: JsonValue
    tools: list[dict[str, JsonValue]] | None
    seed: Seed
    tool_calls: list[dict[str, JsonValue]]
    thinking_blocks: list[dict[str, JsonValue]]
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    logging: LoggingSpec | None


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    if function == "gate":
        return asyncio.run(gate_value(input_value))
    return asyncio.run(plan_value(input_value))


def logger_for(input_value: Input) -> CompressionInterceptionLogger:
    enabled = input_value.get("enabled")
    if enabled is None:
        enabled = True
    return CompressionInterceptionLogger(
        enabled=enabled,
        compression_trigger=SEED_TRIGGER,
        compression_target=SEED_TARGET,
    )


async def gate_value(input_value: Input) -> JsonValue:
    logger = logger_for(input_value)
    messages: list[dict[str, JsonValue]] = []
    kwargs: dict[str, JsonValue] = {}
    ran, ready = await logger.async_should_run_agentic_loop(
        response=input_value.get("response"),
        model="",
        messages=messages,
        tools=input_value.get("tools"),
        stream=False,
        custom_llm_provider="anthropic",
        kwargs=kwargs,
    )
    return {"run": ran, "tools": JSON.validate_python(ready)}


async def seed_cache(logger: CompressionInterceptionLogger, seed: Seed) -> None:
    request: dict[str, JsonValue] = {
        "model": SEED_MODEL,
        "messages": list(SEED_MESSAGES),
        "litellm_call_id": seed["call_id"],
    }
    _ = await logger.async_pre_call_deployment_hook(request, CallTypes.anthropic_messages)


def logging_for(spec: LoggingSpec) -> Logging:
    log = Logging(
        model=SEED_MODEL,
        messages=None,
        stream=False,
        call_type="anthropic_messages",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id=spec["call_id"],
        function_id="b3544",
    )
    loop_params: dict[str, JsonValue] = {}
    agentic_model = spec["agentic_model"]
    if agentic_model is not None:
        loop_params["model"] = agentic_model
    details: dict[str, JsonValue] = {"agentic_loop_params": loop_params}
    log.model_call_details = details
    return log


async def plan_value(input_value: Input) -> JsonValue:
    logger = logger_for(input_value)
    seed = input_value.get("seed")
    if seed is not None:
        await seed_cache(logger, seed)
    tool_calls = input_value.get("tool_calls")
    if tool_calls is None:
        tool_calls = []
    thinking_blocks = input_value.get("thinking_blocks")
    if thinking_blocks is None:
        thinking_blocks = []
    messages = input_value.get("messages")
    if messages is None:
        messages = []
    optional_params = input_value.get("optional_params")
    if optional_params is None:
        optional_params = {}
    kwargs = input_value.get("kwargs")
    if kwargs is None:
        kwargs = {}
    logging_obj: Logging | None = None
    spec = input_value.get("logging")
    if spec is not None:
        logging_obj = logging_for(spec)
    tools: dict[str, list[dict[str, JsonValue]]] = {
        "tool_calls": tool_calls,
        "thinking_blocks": thinking_blocks,
    }
    plan = await logger.async_build_agentic_loop_plan(
        tools=tools,
        model=input_value["model"],
        messages=MESSAGES.validate_python(messages),
        response=None,
        anthropic_messages_provider_config=None,
        anthropic_messages_optional_request_params=optional_params,
        logging_obj=logging_obj,
        stream=False,
        kwargs=kwargs,
    )
    return JSON.validate_json(plan.model_dump_json())
