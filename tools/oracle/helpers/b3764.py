import asyncio
import json
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Final, NotRequired, Protocol, TypedDict

from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.guardrails.guardrail_hooks.compresr.compresr import CompresrGuardrail
from litellm.types.integrations.custom_logger import AgenticLoopPlan
from pydantic import JsonValue, TypeAdapter

JSON: Final = TypeAdapter(JsonValue)
KWARGS: Final = TypeAdapter(dict[str, JsonValue])
MARKERS: Final = TypeAdapter(dict[str, dict[str, list[str]]])
PLACEHOLDER: Final = "$OWN"
NO_STREAM: Final = False
START_TIME: Final = datetime(2024, 1, 1, tzinfo=UTC)

GateCall = Callable[
    [
        JsonValue,
        str,
        list[dict[str, JsonValue]],
        list[dict[str, JsonValue]] | None,
        bool,
        str,
        dict[str, JsonValue],
    ],
    Awaitable[tuple[bool, dict[str, JsonValue]]],
]

PlanCall = Callable[
    [
        dict[str, JsonValue],
        str,
        list[dict[str, JsonValue]],
        JsonValue,
        None,
        dict[str, JsonValue],
        Logging | None,
        bool,
        dict[str, JsonValue],
    ],
    Awaitable[AgenticLoopPlan],
]


class Seed(TypedDict):
    scope: str
    call_id: str
    originals: dict[str, str]


class Call(TypedDict):
    scope: str
    call_id: str
    model: str
    messages: list[dict[str, JsonValue]]
    response: JsonValue
    request_tools: list[dict[str, JsonValue]] | None
    optional_params: dict[str, JsonValue]
    kwargs: dict[str, JsonValue]
    agentic_loop_params: NotRequired[dict[str, JsonValue]]


class Input(TypedDict):
    seed: Seed
    call: Call


class Hooks(Protocol):
    @property
    def async_should_run_agentic_loop(self) -> GateCall: ...

    @property
    def async_build_agentic_loop_plan(self) -> PlanCall: ...


class LoggingDetails(Protocol):
    model_call_details: dict[str, object]


class SeededGuardrail(CompresrGuardrail):
    def seed(self, store_key: str, originals: dict[str, str]) -> None:
        self._store_originals(store_key, originals)


def logging_for(call: Call) -> Logging:
    logging_obj = Logging(
        model=call["model"],
        messages=None,
        stream=NO_STREAM,
        call_type="acompletion",
        start_time=START_TIME,
        litellm_call_id=call["call_id"],
        function_id="b3764",
    )
    details: LoggingDetails = logging_obj
    details.model_call_details.update(
        {"litellm_params": {"metadata": {"user_api_key_auth": UserAPIKeyAuth(api_key=call["scope"])}}}
    )
    params = call.get("agentic_loop_params")
    if params is not None:
        details.model_call_details.update({"agentic_loop_params": params})
    return logging_obj


def own_marker(guardrail: SeededGuardrail) -> str:
    data: dict[str, object] = {"metadata": {}}
    guardrail.mark_pre_call_hook_ran(data)
    return MARKERS.validate_python(data)["metadata"]["_pre_call_executed_guardrails"][0]


async def execute(request_input: Input) -> JsonValue:
    guardrail = SeededGuardrail(api_key="key", guardrail_name="compresr")
    seed = request_input["seed"]
    guardrail.seed(f"{seed['scope']}\x00{seed['call_id']}", seed["originals"])
    hooks: Hooks = guardrail
    call = request_input["call"]
    marker = own_marker(guardrail)
    placed = json.dumps(call["kwargs"]).replace(json.dumps(PLACEHOLDER), json.dumps(marker))
    kwargs = KWARGS.validate_json(placed)
    run, tools = await hooks.async_should_run_agentic_loop(
        call["response"], call["model"], call["messages"], call["request_tools"], NO_STREAM, "openai", kwargs
    )
    plan = await hooks.async_build_agentic_loop_plan(
        tools,
        call["model"],
        call["messages"],
        call["response"],
        None,
        call["optional_params"],
        logging_for(call),
        NO_STREAM,
        kwargs,
    )
    answer: JsonValue = {"gate": {"run": run, "tools": tools}, "plan": JSON.validate_json(plan.model_dump_json())}
    return JSON.validate_json(json.dumps(answer).replace(json.dumps(marker), json.dumps(PLACEHOLDER)))


def run(request_input: Input) -> JsonValue:
    return asyncio.run(execute(request_input))
