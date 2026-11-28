import asyncio
import json
from dataclasses import asdict
from typing import Final, Literal, Protocol, cast

import litellm
from litellm.llms.anthropic.pass_through.adapters import handler
from litellm.llms.anthropic.pass_through.context_management import (
    PolyfillResult,
    apply_context_management,
    errors,
)
from litellm.llms.anthropic.pass_through.context_management.constants import (
    COMPACT_SUMMARY_MAX_TOKENS,
)
from litellm.llms.anthropic.pass_through.context_management.editors import compact
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

ANTHROPIC_ERROR: Final = "AnthropicContextManagementError"
ERROR_TEXT: Final = "broken"
SHAPE: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
RECORD: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Acompletion(Protocol):
    async def __call__(self, **kwargs: JsonValue) -> JsonValue: ...


class CounterFn(Protocol):
    def __call__(self, **kwargs: JsonValue) -> int: ...


class RouterLike(Protocol):
    async def acompletion(self, **kwargs: JsonValue) -> JsonValue: ...


# the real completion and token counter the scenario's fakes stand in front of
REAL_ACOMPLETION: Final[Acompletion] = cast(Acompletion, litellm.acompletion)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py
REAL_COUNTER: Final[CounterFn] = cast(CounterFn, litellm.token_counter)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py


class Dispatch(Protocol):
    async def __call__(self, **kwargs: JsonValue) -> PolyfillResult: ...


REAL_DISPATCH: Final[Dispatch] = cast(Dispatch, apply_context_management)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/dispatcher.py#L72


class Principal(BaseModel):
    model_config = ConfigDict(extra="ignore")

    allowed_model_region: str | None = None


class Scripted(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages: list[dict[str, JsonValue]]
    system: str | list[dict[str, JsonValue]] | None = None


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str = "main-model"
    messages: list[dict[str, JsonValue]]
    system: str | list[dict[str, JsonValue]] | None = None
    tools: list[JsonValue] | None = None
    context_management: dict[str, JsonValue] | list[JsonValue] | None = None
    additional_drop_params: JsonValue = None
    metadata: dict[str, JsonValue] | None = None
    principal: dict[str, JsonValue] | None = None
    router: bool = False
    dispatcher: Literal["real", "result", "null", "error"] = "real"
    scripted: Scripted | None = None
    summary_model: str | None = None
    max_tokens: int = COMPACT_SUMMARY_MAX_TOKENS
    counter: int = 50_001


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    scenarios: list[Scenario]


class Prepare(Protocol):
    async def __call__(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L92-L103
        self,
        *,
        model: str,
        messages: list[dict[str, JsonValue]],
        tools: list[JsonValue] | None,
        system: str | list[dict[str, JsonValue]] | None,
        context_management_spec: dict[str, JsonValue] | list[JsonValue] | None,
        litellm_metadata: dict[str, JsonValue] | None,
        additional_drop_params: JsonValue,
        llm_router: RouterLike | None,
        user_api_key_auth: Principal | None,
    ) -> PolyfillResult | None: ...


PREPARE: Final[Prepare] = cast(Prepare, handler._prepare_context_managed_request)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/adapters/handler.py#L92


class Setting:
    def __init__(self, value: str | int | None) -> None:
        self.value = value

    def __call__(self) -> str | int | None:
        return self.value


class SummaryCall(BaseModel):
    model: str
    messages: list[dict[str, JsonValue]]
    max_tokens: int
    timeout: float
    litellm_metadata: dict[str, JsonValue]


class Deployment:
    def __init__(self, api_base: str) -> None:
        self.api_base = api_base

    async def acompletion(self, **kwargs: JsonValue) -> JsonValue:
        call = SummaryCall.model_validate(kwargs)
        request: dict[str, JsonValue] = {
            "model": "openai/gpt-4o-mini",
            "messages": [*call.messages],
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


# the wrapper's dispatcher call as the recorder saw it, before it runs anything
class Recorder:
    def __init__(self, scenario: Scenario) -> None:
        self.scenario = scenario
        self.calls: list[JsonValue] = []

    async def __call__(self, **kwargs: JsonValue) -> PolyfillResult | None:
        payload: dict[str, JsonValue] = {
            "model": kwargs["model"],
            "messages": kwargs["messages"],
            "tools": kwargs["tools"],
            "system": kwargs["system"],
            "spec": kwargs["context_management_spec"],
            "metadata": kwargs["litellm_metadata"],
            "has_router": kwargs["llm_router"] is not None,
            "has_principal": kwargs["user_api_key_auth"] is not None,
        }
        self.calls.append(RECORD.validate_python(payload))
        if self.scenario.dispatcher == "error":
            raise RuntimeError(ERROR_TEXT)
        if self.scenario.dispatcher == "null":
            return None
        if self.scenario.dispatcher == "result":
            scripted = self.scenario.scripted
            if scripted is None:
                raise RuntimeError("the scripted dispatcher needs a scripted result")
            return PolyfillResult(messages=scripted.messages, system=scripted.system)
        return await REAL_DISPATCH(**kwargs)


def patch_settings(scenario: Scenario) -> None:
    object.__setattr__(compact, "_read_summary_model_setting", Setting(scenario.summary_model))
    object.__setattr__(compact, "_read_summary_max_tokens_setting", Setting(scenario.max_tokens))


async def run_scenario(scenario: Scenario, api_base: str) -> dict[str, JsonValue]:
    patch_settings(scenario)
    object.__setattr__(litellm, "token_counter", ScriptedCounter(scenario.counter))
    deployment = Deployment(api_base)
    object.__setattr__(litellm, "acompletion", deployment.acompletion)
    recorder = Recorder(scenario)
    object.__setattr__(handler, "apply_context_management", recorder)
    principal = None if scenario.principal is None else Principal.model_validate(scenario.principal)
    router: RouterLike | None = deployment if scenario.router else None
    result: dict[str, JsonValue] | None = None
    failure: dict[str, JsonValue] | None = None
    try:
        polyfilled = await PREPARE(
            model=scenario.model,
            messages=scenario.messages,
            tools=scenario.tools,
            system=scenario.system,
            context_management_spec=scenario.context_management,
            litellm_metadata=scenario.metadata,
            additional_drop_params=scenario.additional_drop_params,
            llm_router=router,
            user_api_key_auth=principal,
        )
        if polyfilled is not None:
            result = SHAPE.validate_python(cast(dict[str, JsonValue], asdict(polyfilled)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/result.py#L21
    except errors.AnthropicContextManagementError as error:
        failure_payload: dict[str, JsonValue] = {
            "class": ANTHROPIC_ERROR,
            "status_code": error.status_code,
            "message": error.message,
        }
        failure = SHAPE.validate_python(failure_payload)
    finally:
        object.__setattr__(handler, "apply_context_management", REAL_DISPATCH)
        object.__setattr__(litellm, "token_counter", REAL_COUNTER)
        object.__setattr__(litellm, "acompletion", REAL_ACOMPLETION)
    payload: dict[str, JsonValue] = {
        "result": result,
        "failure": failure,
        "calls": recorder.calls,
    }
    return SHAPE.validate_python(payload)


class ContextPreparationRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    messages_context_preparation: Call

    async def dispatch(self, api_base: str) -> list[JsonValue]:
        scenarios = self.messages_context_preparation.scenarios
        return [await run_scenario(scenario, api_base) for scenario in scenarios]

    def serialize(self, api_base: str) -> str:
        runs = asyncio.run(self.dispatch(api_base))
        payload: dict[str, JsonValue] = {"messages_context_preparation": {"value": {"runs": runs}}}
        return json.dumps(payload)
