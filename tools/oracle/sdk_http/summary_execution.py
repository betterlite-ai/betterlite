import asyncio
import dataclasses
import json
import logging
from typing import Final, Literal, Protocol, cast

import litellm
from litellm import Router
from litellm.llms.anthropic.pass_through.context_management import (
    PolyfillResult,
    errors,
)
from litellm.llms.anthropic.pass_through.context_management.constants import (
    COMPACT_SUMMARY_MAX_TOKENS,
)
from litellm.llms.anthropic.pass_through.context_management.editors import (
    apply_compact_20260112,
    compact,
)
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.proxy_server import general_settings
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

MODULE_PREFIX: Final = "compact_20260112:"
CALL_FAILED_PREFIX: Final = "compact_20260112: summary call failed:"
CALL_FAILED: Final = "compact_20260112: summary call failed"
COUNT_FAILED_PREFIX: Final = "compact_20260112: token_counter failed; assuming under threshold:"
COUNT_FAILED: Final = "compact_20260112: token_counter failed; assuming under threshold"
SHAPE: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Acompletion(Protocol):
    async def __call__(self, **kwargs: object) -> object: ...


class CounterFn(Protocol):
    def __call__(self, **kwargs: object) -> int: ...


# the real completion and token counter the scenario's fakes stand in front of
REAL_ACOMPLETION: Final[Acompletion] = cast(Acompletion, litellm.acompletion)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py
REAL_COUNTER: Final[CounterFn] = cast(CounterFn, litellm.token_counter)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py


class Policy(BaseModel):
    model_config = ConfigDict(extra="forbid")

    access: Literal["allow", "deny"] = "allow"
    budget: Literal["allow", "deny"] = "allow"
    rate: Literal["allow", "deny"] = "allow"


class CounterScript(BaseModel):
    model_config = ConfigDict(extra="forbid")

    conversation: int = 150_001
    text: int = 0
    fail: bool = False


class Scenario(BaseModel):
    model_config = ConfigDict(extra="forbid")

    model: str
    messages: list[dict[str, object]]
    system: str | list[dict[str, object]] | None = None
    tools: list[dict[str, object]] | None = None
    edit_spec: dict[str, object]
    metadata: dict[str, object] | None = None
    summary_model: str | None = None
    max_tokens: int = COMPACT_SUMMARY_MAX_TOKENS
    principal: dict[str, object] | None = None
    policy: Policy = Field(default_factory=Policy)
    counter: CounterScript = Field(default_factory=CounterScript)
    route: Literal["router", "direct"] = "router"
    record_steps: bool = False


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    scenarios: list[Scenario]


class Principal(BaseModel):
    model_config = ConfigDict(extra="allow")

    allowed_model_region: str | None = None


class SummaryCall(BaseModel):
    model: str
    messages: list[dict[str, object]]
    max_tokens: int
    timeout: float
    litellm_metadata: dict[str, object]
    user: str | None = None
    allowed_model_region: str | None = None

    def record(self, route: str) -> dict[str, object]:
        return {
            "route": route,
            "model": self.model,
            "messages": self.messages,
            "max_tokens": self.max_tokens,
            "timeout_seconds": int(self.timeout),
            "metadata": self.litellm_metadata,
            "user": self.user,
            "region": self.allowed_model_region,
        }


class Deployment:
    def __init__(self, api_base: str, route: str) -> None:
        self.api_base = api_base
        self.route = route
        self.calls: list[dict[str, object]] = []

    async def acompletion(self, **kwargs: object) -> object:
        call = SummaryCall.model_validate(kwargs)
        self.calls.append(call.record(self.route))
        return await self.complete(call)

    async def __call__(self, **kwargs: object) -> object:
        call = SummaryCall.model_validate(kwargs)
        self.calls.append(call.record(self.route))
        return await self.complete(call)

    async def complete(self, call: SummaryCall) -> object:
        optional: dict[str, object] = {}
        if call.user is not None:
            optional["user"] = call.user
        return await REAL_ACOMPLETION(
            model="openai/gpt-4o-mini",
            messages=call.messages,
            max_tokens=call.max_tokens,
            timeout=call.timeout,
            litellm_metadata=call.litellm_metadata,
            num_retries=0,
            api_base=self.api_base,
            api_key="sk-test",
            **optional,
        )


class PolicyCheck:
    def __init__(self, policy: Policy, record: bool) -> None:
        self.allowed = {
            "access": policy.access,
            "budget": policy.budget,
            "rate": policy.rate,
        }
        self.record = record
        self.steps: list[str] = []

    def note(self, step: str) -> bool:
        if self.record:
            self.steps.append(step)
        return self.allowed[step] == "allow"

    async def access(
        self,
        user_api_key_auth: UserAPIKeyAuth | None,
        summary_model: str,
        llm_router: Router | None,
    ) -> bool:
        del user_api_key_auth, summary_model, llm_router
        return self.note("access")

    async def budget(
        self,
        user_api_key_auth: UserAPIKeyAuth | None,
        summary_model: str,
    ) -> bool:
        del user_api_key_auth, summary_model
        return self.note("budget")

    async def rate_limit(
        self,
        user_api_key_auth: UserAPIKeyAuth | None,
        summary_model: str,
    ) -> bool:
        del user_api_key_auth, summary_model
        return self.note("rate")


class ScriptedCounter:
    def __init__(self, script: CounterScript) -> None:
        self.conversation = script.conversation
        self.text = script.text
        self.fail = script.fail

    def __call__(
        self,
        model: str = "",
        text: str | None = None,
        messages: object = None,
        tools: object = None,
        **kwargs: object,
    ) -> int:
        del model, messages, tools, kwargs
        if self.fail:
            raise ValueError("broken")
        if text is None:
            return self.conversation
        return self.text


class LogCapture(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.lines: list[dict[str, object]] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(MODULE_PREFIX):
            return
        self.lines.append(
            {"level": record.levelname.lower(), "message": normalize(message)}
        )


def normalize(message: str) -> str:
    if message.startswith(CALL_FAILED_PREFIX):
        return CALL_FAILED
    if message.startswith(COUNT_FAILED_PREFIX):
        return COUNT_FAILED
    return message


def apply_settings(scenario: Scenario) -> None:
    general_settings.update(
        {
            "context_management_summary_model": scenario.summary_model,
            "context_management_summary_max_tokens": scenario.max_tokens,
        }
    )


def patch_checks(policy: PolicyCheck) -> None:
    object.__setattr__(compact, "_check_summary_model_access", policy.access)
    object.__setattr__(compact, "_check_summary_model_budget", policy.budget)
    object.__setattr__(compact, "_check_summary_model_rate_limit", policy.rate_limit)


def shape(result: PolyfillResult) -> dict[str, JsonValue]:
    return SHAPE.validate_python(dataclasses.asdict(result))


def failure(error: errors.AnthropicContextManagementError) -> dict[str, object]:
    return {"status_code": error.status_code, "message": error.message}


async def call_editor(
    scenario: Scenario,
    llm_router: Router | None,
    principal: UserAPIKeyAuth | None,
) -> PolyfillResult:
    return await apply_compact_20260112(
        model=scenario.model,
        messages=scenario.messages,
        tools=scenario.tools,
        system=scenario.system,
        edit_spec=scenario.edit_spec,
        litellm_metadata=scenario.metadata,
        llm_router=llm_router,
        user_api_key_auth=principal,
    )


async def run_scenario(scenario: Scenario, api_base: str) -> dict[str, object]:
    apply_settings(scenario)
    policy = PolicyCheck(scenario.policy, scenario.record_steps)
    patch_checks(policy)
    object.__setattr__(litellm, "token_counter", ScriptedCounter(scenario.counter))
    deployment = Deployment(api_base, scenario.route)
    llm_router: Router | None = None
    if scenario.route == "router":
        llm_router = cast(Router, deployment)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py
        object.__setattr__(litellm, "acompletion", REAL_ACOMPLETION)
    else:
        object.__setattr__(litellm, "acompletion", deployment)
    principal = (
        None
        if scenario.principal is None
        else cast(UserAPIKeyAuth, Principal.model_validate(scenario.principal))  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py
    )
    capture = LogCapture()
    logger = logging.getLogger("LiteLLM")
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [capture]
    logger.setLevel(logging.DEBUG)
    try:
        result = shape(await call_editor(scenario, llm_router, principal))
        refused = None
    except errors.AnthropicContextManagementError as error:
        result = None
        refused = failure(error)
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
        object.__setattr__(litellm, "token_counter", REAL_COUNTER)
    return {
        "result": result,
        "failure": refused,
        "calls": deployment.calls,
        "steps": policy.steps if scenario.record_steps else [],
        "diagnostics": capture.lines,
    }


async def dispatch(call: Call, api_base: str) -> list[dict[str, object]]:
    runs: list[dict[str, object]] = []
    for scenario in call.scenarios:
        runs.append(await run_scenario(scenario, api_base))
    return runs


class SummaryExecutionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    summary_execution: Call

    def serialize(self, api_base: str) -> str:
        runs = asyncio.run(dispatch(self.summary_execution, api_base))
        return json.dumps({"summary_execution": {"value": {"runs": runs}}})
