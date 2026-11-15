import asyncio
import json
from typing import Literal, TypedDict

import litellm
from litellm.exceptions import APIConnectionError
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.anthropic_messages.transformation import BaseAnthropicMessagesConfig
from litellm.types.integrations.custom_logger import AgenticLoopPlan
from openai.types.responses.response_create_params import ResponseInputParam, ToolParam
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter
from pydantic import JsonValue as Json
from pydantic_core import to_jsonable_python
from typing_extensions import override

INPUT: TypeAdapter[str | ResponseInputParam] = TypeAdapter(str | ResponseInputParam)
PLAN: TypeAdapter[AgenticLoopPlan] = TypeAdapter(AgenticLoopPlan)
JSON: TypeAdapter[Json] = TypeAdapter(Json)


class Identity(TypedDict):
    created_at: int
    status: str


IDENTITY: TypeAdapter[Identity] = TypeAdapter(Identity)


class RequestParams(TypedDict, total=False):
    stream: bool
    temperature: float
    tools: list[ToolParam]


class RequestKwargs(TypedDict, total=False):
    custom_llm_provider: str
    _agentic_loop_depth: int
    max_agentic_loops: int
    _agentic_loop_fingerprints: list[str]


class GateScript(BaseModel):
    model_config = ConfigDict(extra="forbid")
    run: bool
    tools: dict[str, Json] = Field(default_factory=dict)


class CallbackScript(BaseModel):
    model_config = ConfigDict(extra="forbid")
    name: str
    skip: bool = False
    gate: GateScript | None = None
    gate_error: str | None = None
    plan: dict[str, Json] | None = None
    plan_error: str | None = None


class ContinuationCall(BaseModel):
    model_config = ConfigDict(extra="forbid")
    model: str
    input: Json
    params: RequestParams = Field(default_factory=dict)
    kwargs: RequestKwargs = Field(default_factory=dict)
    callbacks: list[CallbackScript] = Field(default_factory=list)
    dynamic: list[CallbackScript] = Field(default_factory=list)


class GateCallback(CustomLogger):
    def __init__(self, script: CallbackScript) -> None:
        self.script = script

    @override
    async def async_should_run_agentic_loop(
        self,
        # ast-grep-ignore: no-loose-types
        response: object,
        model: str,
        messages: list[dict[str, Json]],
        tools: list[dict[str, Json]] | None,
        stream: bool,
        custom_llm_provider: str,
        kwargs: dict[str, Json],
    ) -> tuple[bool, dict[str, Json]]:
        if self.script.gate_error is not None:
            raise ValueError(self.script.gate_error)
        gate = self.script.gate
        if gate is None:
            return False, {}
        return gate.run, gate.tools


class PlanCallback(GateCallback):
    @override
    async def async_build_agentic_loop_plan(
        self,
        tools: dict[str, Json],
        model: str,
        messages: list[dict[str, Json]],
        # ast-grep-ignore: no-loose-types
        response: object,
        # ast-grep-ignore: no-long-names
        anthropic_messages_provider_config: BaseAnthropicMessagesConfig | None,
        # ast-grep-ignore: no-long-names
        anthropic_messages_optional_request_params: dict[str, Json],
        logging_obj: Logging,
        stream: bool,
        kwargs: dict[str, Json],
    ) -> AgenticLoopPlan:
        if self.script.plan_error is not None:
            raise ValueError(self.script.plan_error)
        plan = self.script.plan
        return AgenticLoopPlan() if plan is None else PLAN.validate_python(plan)


def callback(script: CallbackScript) -> CustomLogger:
    if script.plan is not None or script.plan_error is not None:
        return PlanCallback(script)
    return GateCallback(script)


def global_callbacks(scripts: list[CallbackScript]) -> list[CustomLogger | Literal["langfuse"]]:
    entries: list[CustomLogger | Literal["langfuse"]] = []
    for script in scripts:
        if script.skip:
            entries.append("langfuse")
            continue
        entries.append(callback(script))
    return entries


async def execute(call: ContinuationCall, api_base: str) -> Json:
    litellm.callbacks = [*global_callbacks(call.callbacks)]
    dynamic = [callback(script) for script in call.dynamic]
    outcome: dict[str, Json] = {"returned": None, "error": None}
    try:
        result = await litellm.aresponses(
            model=call.model,
            input=INPUT.validate_python(call.input),
            api_base=api_base,
            api_key="sk-test",
            success_callback=dynamic,
            **call.params,
            **call.kwargs,
        )
    except APIConnectionError as error:
        failure: Json = {"class": type(error).__name__, "message": str(error.__context__)}
        outcome["error"] = failure
        return outcome
    outcome["returned"] = JSON.validate_python(IDENTITY.validate_python(to_jsonable_python(result)))
    return outcome


def run_continuation(call: ContinuationCall, api_base: str) -> str:
    outcome = asyncio.run(execute(call, api_base))
    return json.dumps({"responses_agentic_continuation": {"value": outcome}})
