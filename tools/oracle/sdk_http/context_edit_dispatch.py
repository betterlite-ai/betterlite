import asyncio
import json
from dataclasses import asdict
from typing import Final, Protocol, cast

import litellm
from litellm.llms.anthropic.pass_through.context_management import (
    PolyfillResult,
    apply_context_management,
    errors,
)
from litellm.llms.anthropic.pass_through.context_management.constants import (
    COMPACT_SUMMARY_MAX_TOKENS,
)
from litellm.llms.anthropic.pass_through.context_management.editors import compact
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter

ANTHROPIC_ERROR: Final = "AnthropicContextManagementError"
SHAPE: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Acompletion(Protocol):
    async def __call__(self, **kwargs: JsonValue) -> JsonValue: ...


class CounterFn(Protocol):
    def __call__(self, **kwargs: JsonValue) -> int: ...


# the real completion and token counter the scenario's fakes stand in front of
REAL_ACOMPLETION: Final[Acompletion] = cast(Acompletion, litellm.acompletion)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py
REAL_COUNTER: Final[CounterFn] = cast(CounterFn, litellm.token_counter)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py


class CounterScript(BaseModel):
    model_config = ConfigDict(extra="forbid")

    count: int = 50_001


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str = "main-model"
    messages: list[dict[str, JsonValue]]
    system: str | list[dict[str, JsonValue]] | None = None
    tools: list[JsonValue] | None = None
    context_management: dict[str, JsonValue] | list[JsonValue] | None = None
    summary_model: str | None = None
    max_tokens: int = COMPACT_SUMMARY_MAX_TOKENS
    counter: CounterScript = Field(default_factory=CounterScript)


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    scenarios: list[Scenario]


# the dispatcher's own keyword arguments; the two request-wide values stay absent for a case without a proxy
class Dispatch(Protocol):
    async def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L72-L82
        self,
        *,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[JsonValue] | None,
        system: str | list[dict[str, JsonValue]] | None,
        context_management_spec: dict[str, JsonValue] | list[JsonValue] | None,
        litellm_metadata: dict[str, JsonValue] | None = None,
    ) -> PolyfillResult: ...


DISPATCH = cast(Dispatch, apply_context_management)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L72


class Setting:
    def __init__(self, value: str | int | None) -> None:
        self.value = value

    def __call__(self) -> str | int | None:
        return self.value


class SummaryCall(BaseModel):
    model_config = ConfigDict(extra="ignore")

    model: str
    messages: list[dict[str, JsonValue]]
    max_tokens: int
    timeout: float
    litellm_metadata: dict[str, JsonValue]


class Deployment:
    def __init__(self, api_base: str) -> None:
        self.api_base = api_base

    async def __call__(self, **kwargs: JsonValue) -> JsonValue:
        call = SummaryCall.model_validate(kwargs)
        request: dict[str, JsonValue] = {
            "model": f"openai/{call.model}",
            "messages": call.messages,
            "max_tokens": call.max_tokens,
            "timeout": call.timeout,
            "litellm_metadata": call.litellm_metadata,
            "num_retries": 0,
            "api_base": self.api_base,
            "api_key": "sk-test",
        }
        return await REAL_ACOMPLETION(**request)


class ScriptedCounter:
    def __init__(self, count: int) -> None:
        self.count = count

    def __call__(self, **kwargs: JsonValue) -> int:
        del kwargs
        return self.count


def patch_settings(scenario: Scenario) -> None:
    object.__setattr__(compact, "_read_summary_model_setting", Setting(scenario.summary_model))
    object.__setattr__(compact, "_read_summary_max_tokens_setting", Setting(scenario.max_tokens))


async def call_dispatch(scenario: Scenario) -> PolyfillResult:
    return await DISPATCH(
        model=scenario.model,
        messages=scenario.messages,
        tools=scenario.tools,
        system=scenario.system,
        context_management_spec=scenario.context_management,
        litellm_metadata=None,
    )


async def run_scenario(scenario: Scenario, api_base: str) -> dict[str, JsonValue]:
    patch_settings(scenario)
    object.__setattr__(litellm, "token_counter", ScriptedCounter(scenario.counter.count))
    object.__setattr__(litellm, "acompletion", Deployment(api_base))
    result: dict[str, JsonValue] | None = None
    failure: dict[str, JsonValue] | None = None
    try:
        polyfilled = await call_dispatch(scenario)
        result = SHAPE.validate_python(cast(dict[str, JsonValue], asdict(polyfilled)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/result.py#L21
    except errors.AnthropicContextManagementError as error:
        failure = SHAPE.validate_python(
            {"class": ANTHROPIC_ERROR, "status_code": error.status_code, "message": error.message}
        )
    finally:
        object.__setattr__(litellm, "token_counter", REAL_COUNTER)
        object.__setattr__(litellm, "acompletion", REAL_ACOMPLETION)
    return SHAPE.validate_python({"result": result, "failure": failure})


class ContextEditDispatchRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    context_edit_dispatch: Call

    async def dispatch(self, api_base: str) -> list[JsonValue]:
        scenarios = self.context_edit_dispatch.scenarios
        return [await run_scenario(scenario, api_base) for scenario in scenarios]

    def serialize(self, api_base: str) -> str:
        runs = asyncio.run(self.dispatch(api_base))
        payload: dict[str, JsonValue] = {"context_edit_dispatch": {"value": {"runs": runs}}}
        return json.dumps(payload)
