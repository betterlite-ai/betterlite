import asyncio
import json
import logging
from dataclasses import asdict
from typing import Final, Literal, Protocol, cast

import litellm
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
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.router import Router
from pydantic import BaseModel, ConfigDict, Field, JsonValue, TypeAdapter
from typing_extensions import override

MODULE_PREFIX: Final = "compact_20260112:"
CALL_FAILED_PREFIX: Final = "compact_20260112: summary call failed:"
CALL_FAILED: Final = "compact_20260112: summary call failed"
COUNT_FAILED_PREFIX: Final = "compact_20260112: token_counter failed; assuming under threshold:"
COUNT_FAILED: Final = "compact_20260112: token_counter failed; assuming under threshold"
SHAPE: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])


class Acompletion(Protocol):
    async def __call__(self, **kwargs: JsonValue) -> JsonValue: ...


class CounterFn(Protocol):
    def __call__(self, **kwargs: JsonValue) -> int: ...


# the real completion and token counter the scenario's fakes stand in front of
REAL_ACOMPLETION: Final[Acompletion] = cast(Acompletion, litellm.acompletion)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/main.py
REAL_COUNTER: Final[CounterFn] = cast(CounterFn, litellm.token_counter)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py


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
    messages: list[JsonValue]
    system: str | list[JsonValue] | None = None
    tools: list[JsonValue] | None = None
    edit_spec: dict[str, JsonValue]
    metadata: dict[str, JsonValue] | None = None
    summary_model: str | None = None
    max_tokens: int = COMPACT_SUMMARY_MAX_TOKENS
    principal: dict[str, JsonValue] | None = None
    policy: Policy = Field(default_factory=Policy)
    counter: CounterScript = Field(default_factory=CounterScript)
    route: Literal["router", "direct"] = "router"
    record_steps: bool = False


class Call(BaseModel):
    model_config = ConfigDict(extra="forbid")

    scenarios: list[Scenario]


class SummaryCall(BaseModel):
    model: str
    messages: list[JsonValue]
    max_tokens: int
    timeout: float
    litellm_metadata: dict[str, JsonValue]
    user: str | None = None
    allowed_model_region: str | None = None


class Deployment:
    def __init__(self, api_base: str, route: str) -> None:
        self.api_base = api_base
        self.route = route
        self.calls: list[JsonValue] = []

    async def acompletion(self, **kwargs: JsonValue) -> JsonValue:
        call = SummaryCall.model_validate(kwargs)
        self.calls.append(
            {
                "route": self.route,
                "model": call.model,
                "messages": call.messages,
                "max_tokens": call.max_tokens,
                "timeout_seconds": int(call.timeout),
                "metadata": call.litellm_metadata,
                "user": call.user,
                "region": call.allowed_model_region,
            }
        )
        request: dict[str, JsonValue] = {
            "model": "openai/gpt-4o-mini",
            "messages": call.messages,
            "max_tokens": call.max_tokens,
            "timeout": call.timeout,
            "litellm_metadata": call.litellm_metadata,
            "num_retries": 0,
            "api_base": self.api_base,
            "api_key": "sk-test",
        }
        if call.user is not None:
            request["user"] = call.user
        return await REAL_ACOMPLETION(**request)

    async def __call__(self, **kwargs: JsonValue) -> JsonValue:
        return await self.acompletion(**kwargs)


class PolicyCheck:
    def __init__(self, policy: Policy, *, record: bool) -> None:
        self.allowed = {
            "access": policy.access,
            "budget": policy.budget,
            "rate": policy.rate,
        }
        self.record = record
        self.steps: list[JsonValue] = []

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
        messages: JsonValue = None,
        tools: JsonValue = None,
        **kwargs: JsonValue,
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
        self.lines: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if not message.startswith(MODULE_PREFIX):
            return
        if message.startswith(CALL_FAILED_PREFIX):
            message = CALL_FAILED
        elif message.startswith(COUNT_FAILED_PREFIX):
            message = COUNT_FAILED
        self.lines.append({"level": record.levelname.lower(), "message": message})


def patch_checks(policy: PolicyCheck) -> None:
    object.__setattr__(compact, "_check_summary_model_access", policy.access)
    object.__setattr__(compact, "_check_summary_model_budget", policy.budget)
    object.__setattr__(compact, "_check_summary_model_rate_limit", policy.rate_limit)


async def call_editor(
    scenario: Scenario,
    llm_router: Router | None,
    principal: UserAPIKeyAuth | None,
) -> PolyfillResult:
    return await apply_compact_20260112(
        model=scenario.model,
        messages=cast(list[dict[str, object]], scenario.messages),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1163
        tools=cast(list[dict[str, object]] | None, scenario.tools),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1164
        system=cast(str | list[dict[str, object]] | None, scenario.system),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1165
        edit_spec=cast(dict[str, object], scenario.edit_spec),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L1166
        litellm_metadata=scenario.metadata,
        llm_router=llm_router,
        user_api_key_auth=principal,
    )


async def run_scenario(scenario: Scenario, api_base: str) -> dict[str, JsonValue]:
    updates: dict[str, JsonValue] = {
        "context_management_summary_model": scenario.summary_model,
        "context_management_summary_max_tokens": scenario.max_tokens,
    }
    proxy_server.general_settings = updates
    policy = PolicyCheck(scenario.policy, record=scenario.record_steps)
    patch_checks(policy)
    object.__setattr__(litellm, "token_counter", ScriptedCounter(scenario.counter))
    deployment = Deployment(api_base, scenario.route)
    llm_router: Router | None = None
    if scenario.route == "router":
        llm_router = cast(Router, deployment)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py
        object.__setattr__(litellm, "acompletion", REAL_ACOMPLETION)
    else:
        object.__setattr__(litellm, "acompletion", deployment)
    principal = (
        None
        if scenario.principal is None
        else UserAPIKeyAuth.model_validate(scenario.principal)
    )
    capture = LogCapture()
    logger = logging.getLogger("LiteLLM")
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [capture]
    logger.setLevel(logging.DEBUG)
    outcome: dict[str, JsonValue] | None
    refused: dict[str, JsonValue] | None
    try:
        polyfilled = await call_editor(scenario, llm_router, principal)
        outcome = SHAPE.validate_python(cast(dict[str, JsonValue], asdict(polyfilled)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/result.py#L21
        refused = None
    except errors.AnthropicContextManagementError as error:
        outcome = None
        refused = {"status_code": error.status_code, "message": error.message}
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
        object.__setattr__(litellm, "token_counter", REAL_COUNTER)
    return {
        "result": outcome,
        "failure": refused,
        "calls": deployment.calls,
        "steps": policy.steps if scenario.record_steps else [],
        "diagnostics": capture.lines,
    }


async def dispatch(call: Call, api_base: str) -> list[JsonValue]:
    return [await run_scenario(scenario, api_base) for scenario in call.scenarios]


class SummaryExecutionRequest(BaseModel):
    model_config = ConfigDict(extra="forbid")

    summary_execution: Call

    def serialize(self, api_base: str) -> str:
        runs = asyncio.run(dispatch(self.summary_execution, api_base))
        payload: dict[str, JsonValue] = {"summary_execution": {"value": {"runs": runs}}}
        return json.dumps(payload)
