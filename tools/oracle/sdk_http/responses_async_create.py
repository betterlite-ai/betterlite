import asyncio
import json
from collections.abc import Coroutine
from datetime import UTC, datetime
from typing import Final, Protocol, TypeAlias, TypedDict, cast

import litellm
import openai
from litellm.integrations.anthropic_cache_control_hook import CARRY_UNMATCHED_MESSAGE_POINTS
from litellm.integrations.custom_logger import CustomLogger
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.types.llms.openai import AllMessageValues, ResponsesAPIResponse
from litellm.types.prompts.init_prompts import PromptSpec
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

# an unknown model prints LiteLLM's provider list to stdout, where only the answer belongs
litellm.suppress_debug_info = True

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)

AsyncCreateResult: TypeAlias = Coroutine[object, object, ResponsesAPIResponse]


# litellm's @client decorator erases the wrapper signature, so the protocol names the call
class AsyncCalls(Protocol):
    def aresponses(self, **kwargs: JsonValue | Logging | None) -> AsyncCreateResult: ...


CALLS: Final[AsyncCalls] = cast(AsyncCalls, litellm)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/responses/main.py#L575-L609


class HookAnswer(TypedDict):
    model: str
    messages: list[JsonValue]
    optional_params: dict[str, JsonValue]


class HookScript(TypedDict):
    answer: HookAnswer


class HookCall(TypedDict):
    model: str
    prompt_id: str | None
    prompt_label: str | None
    prompt_version: JsonValue
    prompt_variables: JsonValue
    messages: JsonValue
    carry: bool
    aresponses: bool


class Observation(TypedDict):
    checks: int
    ran: int
    sync_checks: int
    calls: list[HookCall]


class AsyncRun(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    input: JsonValue
    include: JsonValue = None
    instructions: str | None = None
    max_output_tokens: int | None = None
    prompt: JsonValue = None
    metadata: JsonValue = None
    parallel_tool_calls: bool | None = None
    previous_response_id: str | None = None
    reasoning: JsonValue = None
    store: bool | None = None
    background: bool | None = None
    stream: bool | None = None
    temperature: float | None = None
    text: JsonValue = None
    text_format: JsonValue = None
    tool_choice: JsonValue = None
    tools: JsonValue = None
    top_p: float | None = None
    truncation: str | None = None
    user: str | None = None
    service_tier: str | None = None
    safety_identifier: str | None = None
    extra_headers: dict[str, JsonValue] | None = None
    extra_query: JsonValue = None
    extra_body: JsonValue = None
    timeout_seconds: float | None = None
    hook: HookScript | None = None
    kwargs: dict[str, JsonValue] = Field(default_factory=dict)


class ResponsesAsyncCreateCall(BaseModel):
    model_config = ConfigDict(extra="forbid")

    runs: list[AsyncRun]


def call_kwargs(run: AsyncRun, api_base: str) -> dict[str, JsonValue]:
    values: dict[str, JsonValue] = {
        "model": run.model,
        "input": run.input,
        "include": run.include,
        "instructions": run.instructions,
        "max_output_tokens": run.max_output_tokens,
        "prompt": run.prompt,
        "metadata": run.metadata,
        "parallel_tool_calls": run.parallel_tool_calls,
        "previous_response_id": run.previous_response_id,
        "reasoning": run.reasoning,
        "store": run.store,
        "background": run.background,
        "stream": run.stream,
        "temperature": run.temperature,
        "text": run.text,
        "text_format": run.text_format,
        "tool_choice": run.tool_choice,
        "tools": run.tools,
        "top_p": run.top_p,
        "truncation": run.truncation,
        "user": run.user,
        "service_tier": run.service_tier,
        "safety_identifier": run.safety_identifier,
        "extra_headers": run.extra_headers,
        "extra_query": run.extra_query,
        "extra_body": run.extra_body,
        "timeout": run.timeout_seconds,
        "api_base": api_base,
    }
    values.update(run.kwargs)
    return values


# The logging object aresponses looks for, scripted and counted the way the native port counts it
class ScriptedLogging(Logging):
    script: HookAnswer
    recording: Observation

    def __init__(self, script: HookAnswer) -> None:
        super().__init__(model="", messages="", stream=False, call_type="responses", start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="call-1", function_id="fn-1")  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L556  # noqa: E501
        self.script = script
        self.recording = {"checks": 0, "ran": 0, "sync_checks": 0, "calls": []}

    @override
    # ast-grep-ignore: no-long-names
    def should_run_prompt_management_hooks(
        self,
        non_default_params: dict[str, JsonValue],
        prompt_id: str | None = None,
        tools: list[dict[str, JsonValue]] | None = None,
    ) -> bool:
        runs = super().should_run_prompt_management_hooks(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L1012
            non_default_params=non_default_params, prompt_id=prompt_id, tools=tools
        )
        self.recording["checks"] += 1
        if runs:
            self.recording["ran"] += 1
        return runs

    @override
    def get_chat_completion_prompt(
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        prompt_variables: dict[str, JsonValue] | None,
        prompt_id: str | None = None,
        prompt_spec: PromptSpec | None = None,
        prompt_management_logger: CustomLogger | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        # ast-grep-ignore: no-loose-types
        request_kwargs: dict[str, object] | None = None,
        injected_for_every_deployment: bool = False,
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        del non_default_params, prompt_variables, prompt_id, prompt_spec
        del prompt_management_logger, prompt_label, prompt_version, request_kwargs
        del injected_for_every_deployment
        self.recording["sync_checks"] += 1
        return model, messages, {}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005

    @override
    # ast-grep-ignore: no-long-names
    async def async_get_chat_completion_prompt(
        self,
        model: str,
        messages: list[AllMessageValues],
        non_default_params: dict[str, JsonValue],
        prompt_variables: dict[str, JsonValue] | None,
        prompt_id: str | None = None,
        prompt_spec: PromptSpec | None = None,
        prompt_management_logger: CustomLogger | None = None,
        tools: list[dict[str, JsonValue]] | None = None,
        prompt_label: str | None = None,
        prompt_version: int | None = None,
        # ast-grep-ignore: no-loose-types
        request_kwargs: dict[str, object] | None = None,
        injected_for_every_deployment: bool = False,
    ) -> tuple[str, list[AllMessageValues], dict[str, JsonValue]]:
        del prompt_spec, prompt_management_logger, tools, injected_for_every_deployment
        script = self.script
        carried = request_kwargs is not None and CARRY_UNMATCHED_MESSAGE_POINTS in request_kwargs
        call: HookCall = {
            "model": model,
            "prompt_id": prompt_id,
            "prompt_label": prompt_label,
            "prompt_version": prompt_version,
            "prompt_variables": prompt_variables,
            "messages": JSON.validate_python(messages),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
            "carry": carried,
            "aresponses": bool(non_default_params.get("aresponses")),
        }
        self.recording["calls"].append(call)
        merged = cast(list[AllMessageValues], script["messages"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005
        return script["model"], merged, dict(script["optional_params"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L998-L1005


def called(run: AsyncRun, api_base: str, scripted: ScriptedLogging | None) -> AsyncCreateResult:
    kwargs = call_kwargs(run, api_base)
    if scripted is None:
        return CALLS.aresponses(**kwargs)
    return CALLS.aresponses(**kwargs, litellm_logging_obj=scripted)


def observation(scripted: ScriptedLogging | None) -> JsonValue:
    if scripted is None:
        empty: Observation = {"checks": 0, "ran": 0, "sync_checks": 0, "calls": []}
        return JSON.validate_python(empty)
    return JSON.validate_python(scripted.recording)


def status_code(error: Exception) -> int:
    match error:
        case openai.APIStatusError():
            return error.status_code
        case openai.APIConnectionError():
            return 500
        case object():
            return 0


# litellm builds its exceptions without a body, so the SDK reads no type, param or code
def failure(error: Exception) -> dict[str, JsonValue]:
    return {
        "class": type(error).__name__,
        "status": status_code(error),
        "type": None,
        "param": None,
        "code": None,
        "message": str(error),
    }


def run_outcome(run: AsyncRun, api_base: str) -> JsonValue:
    scripted = None if run.hook is None else ScriptedLogging(run.hook["answer"])
    try:
        with asyncio.Runner() as runner:
            response = runner.run(called(run, api_base, scripted))
        outcome: dict[str, JsonValue] = {
            "result": JSON.validate_json(response.model_dump_json()),
            "failure": None,
        }
    except (ValueError, openai.OpenAIError) as error:
        outcome = {"result": None, "failure": failure(error)}
    outcome["hooks"] = observation(scripted)
    return JSON.validate_python(outcome)


class ResponsesAsyncCreateRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    responses_async_create: ResponsesAsyncCreateCall

    def serialize(self, api_base: str) -> str:
        runs: list[JsonValue] = [
            run_outcome(run, api_base) for run in self.responses_async_create.runs
        ]
        outcome: JsonValue = {"responses_async_create": {"value": {"runs": runs}}}
        return json.dumps(outcome)
