import asyncio
import json
from typing import Literal

import litellm
import openai
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.anthropic_messages.transformation import BaseAnthropicMessagesConfig
from litellm.types.integrations.custom_logger import AgenticLoopPlan
from openai.types.responses.response_input_param import ResponseInputParam
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter, ValidationError
from pydantic import JsonValue as Json
from typing_extensions import override

INPUT: TypeAdapter[str | ResponseInputParam] = TypeAdapter(str | ResponseInputParam)
PLAN: TypeAdapter[AgenticLoopPlan] = TypeAdapter(AgenticLoopPlan)
JSON: TypeAdapter[Json] = TypeAdapter(Json)


class Identity(BaseModel):
    model_config = ConfigDict(extra="ignore")
    created_at: int
    status: str


class ScriptedCallbackError(ValueError):
    pass


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
    params: dict[str, Json] = Field(default_factory=dict)
    kwargs: dict[str, Json] = Field(default_factory=dict)
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
            raise ScriptedCallbackError(self.script.gate_error)
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
            raise ScriptedCallbackError(self.script.plan_error)
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
    except openai.APIConnectionError as error:
        failure: Json = {"class": type(error).__name__, "message": str(error.__context__)}
        outcome["error"] = failure
        return outcome
    # any answered object, dict override included, has the identity; a non-object stays null
    try:
        identity = Identity.model_validate(result, from_attributes=True)
    except ValidationError:
        return outcome
    outcome["returned"] = JSON.validate_json(identity.model_dump_json())
    return outcome


def run_continuation(call: ContinuationCall, api_base: str) -> str:
    returned = asyncio.run(execute(call, api_base))
    outcome: Json = {"responses_agentic_continuation": {"value": returned}}
    return json.dumps(outcome)
