import asyncio
import json
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Final, NotRequired, Protocol, TypeAlias, TypedDict

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

# LiteLLM keeps live objects in these dicts, so they carry more than JsonValue allows
Details: TypeAlias = dict[str, object]

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


class Gate(Protocol):
    @property
    def async_should_run_agentic_loop(self) -> GateCall: ...


class PlanBuilder(Protocol):
    @property
    def async_build_agentic_loop_plan(self) -> PlanCall: ...


class LoggingDetails(Protocol):
    model_call_details: Details


class _ProbeMeta(type):
    pass


class SeededGuardrail(CompresrGuardrail, metaclass=_ProbeMeta):
    # the metaclass keeps mypy's __init_subclass__ check away from LiteLLM's untyped kwargs
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
    auth = UserAPIKeyAuth(api_key=call["scope"])
    params: Details = {"metadata": {"user_api_key_auth": auth}}
    details.model_call_details.update({"litellm_params": params})
    agentic_loop_params = call.get("agentic_loop_params")
    if agentic_loop_params is not None:
        details.model_call_details.update({"agentic_loop_params": agentic_loop_params})
    return logging_obj


# Mirrors _scoped_store_key: without a caller scope the call id alone is the key
def seed_originals(guardrail: SeededGuardrail, seed: Seed) -> None:
    key = seed["call_id"] if not seed["scope"] else f"{seed['scope']}\x00{seed['call_id']}"
    guardrail.seed(key, seed["originals"])


def own_marker(guardrail: SeededGuardrail) -> str:
    data: Details = {"metadata": {}}
    guardrail.mark_pre_call_hook_ran(data)
    markers = MARKERS.validate_python(data)
    return markers["metadata"]["_pre_call_executed_guardrails"][0]


def hydrate_kwargs(call: Call, marker: str) -> dict[str, JsonValue]:
    placed = json.dumps(call["kwargs"]).replace(json.dumps(PLACEHOLDER), json.dumps(marker))
    kwargs = KWARGS.validate_json(placed)
    carrier: JsonValue = {"metadata": {"user_api_key_auth": {"api_key": call["scope"]}}}
    kwargs["litellm_params"] = carrier
    return kwargs


async def gate(
    agent: Gate, call: Call, kwargs: dict[str, JsonValue]
) -> tuple[bool, dict[str, JsonValue]]:
    return await agent.async_should_run_agentic_loop(
        call["response"],
        call["model"],
        call["messages"],
        call["request_tools"],
        NO_STREAM,
        "openai",
        kwargs,
    )


async def plan(
    agent: PlanBuilder,
    call: Call,
    tools: dict[str, JsonValue],
    kwargs: dict[str, JsonValue],
) -> AgenticLoopPlan:
    return await agent.async_build_agentic_loop_plan(
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


async def execute(guardrail: SeededGuardrail, marker: str, request_input: Input) -> JsonValue:
    seed_originals(guardrail, request_input["seed"])
    call = request_input["call"]
    kwargs = hydrate_kwargs(call, marker)
    run, tools = await gate(guardrail, call, kwargs)
    built = await plan(guardrail, call, tools, kwargs)
    plan_json = JSON.validate_json(built.model_dump_json())
    answer: JsonValue = {"gate": {"run": run, "tools": tools}, "plan": plan_json}
    return answer


def run(request_input: Input) -> JsonValue:
    guardrail = SeededGuardrail(api_key="key", guardrail_name="compresr")
    marker = own_marker(guardrail)
    answer = asyncio.run(execute(guardrail, marker, request_input))
    return JSON.validate_json(
        json.dumps(answer).replace(json.dumps(marker), json.dumps(PLACEHOLDER))
    )
