import asyncio
from collections.abc import Awaitable, Callable
from datetime import UTC, datetime
from typing import Final, Protocol

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.custom_httpx.http_handler import AsyncHTTPHandler
from litellm.proxy.guardrails.guardrail_hooks.headroom.headroom import HeadroomGuardrail
from litellm.types.integrations.custom_logger import AgenticLoopPlan
from pydantic import JsonValue, TypeAdapter

from b3775.service import (
    Answer,
    Applier,
    LoggingSpec,
    PlanInput,
    Service,
    ShouldRunInput,
)

JSON: Final = TypeAdapter(JsonValue)
START_TIME: Final = datetime(2024, 1, 1, tzinfo=UTC)
API_BASE: Final = "https://headroom.test"
NO_STREAM: Final = False

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


class Gate(Protocol):
    @property
    def async_should_run_agentic_loop(self) -> GateCall: ...


class PlanBuilder(Protocol):
    @property
    def async_build_agentic_loop_plan(self) -> PlanCall: ...


class AgenticLogging(Protocol):
    model_call_details: dict[str, JsonValue]


def logging_for(spec: LoggingSpec | None) -> Logging | None:
    if spec is None:
        return None
    call_id = spec.get("call_id")
    logging_obj = Logging(
        model="gpt-4o",
        messages=None,
        stream=NO_STREAM,
        call_type="acompletion",
        start_time=START_TIME,
        litellm_call_id="" if call_id is None else call_id,
        function_id="b3775",
    )
    params = spec.get("agentic_loop_params")
    if params is not None:
        seeded: AgenticLogging = logging_obj
        seeded.model_call_details.update({"agentic_loop_params": params})
    return logging_obj


def guardrail(api_key: str | None, timeout: float | None) -> HeadroomGuardrail:
    return HeadroomGuardrail(
        api_base=API_BASE,
        api_key=api_key,
        model=None,
        guardrail_name="headroom",
        unreachable_fallback=None,
        timeout=timeout,
        ccr_retrieval=True,
    )


async def gate(input_value: ShouldRunInput, agent: Gate) -> JsonValue:
    run, tools = await agent.async_should_run_agentic_loop(
        input_value["response"],
        "gpt-4o",
        [],
        input_value["tools"],
        NO_STREAM,
        "openai",
        {},
    )
    result: JsonValue = {"run": run, "tools": JSON.validate_python(tools)}
    return result


async def build_plan(agent: PlanBuilder, input_value: PlanInput) -> AgenticLoopPlan:
    return await agent.async_build_agentic_loop_plan(
        input_value["tools"],
        input_value["model"],
        input_value["messages"],
        input_value["response"],
        None,
        input_value["optional_params"],
        logging_for(input_value["logging"]),
        NO_STREAM,
        input_value["kwargs"],
    )


async def plan_result(input_value: PlanInput, service: Service) -> JsonValue:
    agent = guardrail(input_value["api_key"], input_value["timeout"])
    agent.async_handler = AsyncHTTPHandler(transport=httpx.MockTransport(service))
    applier = Applier(agent)
    seed = input_value["seed"]
    await applier.hook.apply_guardrail(
        {
            "structured_messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "long text"},
                {"role": "assistant", "content": "answer"},
                {"role": "user", "content": "now"},
            ]
        },
        {"litellm_call_id": seed["call_id"]},
        "request",
        None,
    )
    built = await build_plan(agent, input_value)
    result: JsonValue = {
        "plan": JSON.validate_json(built.model_dump_json()),
        "calls": [
            {"method": seen["method"], "path": seen["path"]} for seen in service.calls
        ],
    }
    return result


def plan(input_value: PlanInput) -> JsonValue:
    seed = input_value["seed"]
    messages: list[JsonValue] = [{"role": "user", "content": "short"}]
    body: JsonValue = {"messages": messages, "ccr_hashes": seed["hashes"]}
    compressed: Answer = {"kind": "answer", "status": 200, "body": body}
    service = Service([compressed, *input_value["replies"]])
    return asyncio.run(plan_result(input_value, service))
