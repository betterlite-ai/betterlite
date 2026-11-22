import asyncio
import logging
from collections.abc import Sequence
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.caching.dual_cache import DualCache
from litellm.integrations.custom_logger import CustomLogger
from litellm.llms.anthropic.pass_through.context_management.editors import compact
from litellm.proxy import proxy_server
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.hooks.parallel_request_limiter_v3 import (
    RateLimitDescriptor,
    RateLimitResponse,
    RateLimitUnverifiableError,
)
from litellm.proxy.utils import ProxyLogging
from opentelemetry.trace import Span
from pydantic import JsonValue, TypeAdapter
from typing_extensions import Unpack, override

CHECK_BUDGET: Final = compact._check_summary_model_budget  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L395
CHECK_RATE: Final = compact._check_summary_model_rate_limit  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/context_management/editors/compact.py#L521
PRINCIPALS: Final[TypeAdapter[UserAPIKeyAuth]] = TypeAdapter(UserAPIKeyAuth)

Stage: TypeAlias = Literal["create", "team", "project", "organization", "check"]


class FailureJson(TypedDict):
    stage: Stage
    kind: Literal["unverifiable", "offline"]
    detail: str


class RateScript(TypedDict):
    hook: Literal["limiter", "none", "raises"]
    missing: list[str]
    base: list[RateLimitDescriptor]
    team: list[RateLimitDescriptor]
    project: list[RateLimitDescriptor]
    organization: list[RateLimitDescriptor]
    overall_code: str
    failure: FailureJson | None


class Scenario(TypedDict):
    principal: dict[str, JsonValue] | None
    spend: dict[str, float]
    offline: list[str]
    rate: RateScript


class Input(TypedDict):
    summary_model: str
    scenarios: list[Scenario]


class LimitsJson(TypedDict):
    requests_per_unit: int | None
    tokens_per_unit: int | None
    max_parallel_requests: int | None
    window_size: int | None


class DescriptorJson(TypedDict):
    key: str
    value: str
    rate_limit: LimitsJson | None


class CallJson(TypedDict):
    call: str
    hook: str | None
    model: str | None
    token: str | None
    rpm_limit_type: JsonValue
    tpm_limit_type: JsonValue
    model_has_failures: bool | None
    descriptors: list[DescriptorJson] | None
    parent_otel_span: Literal["set"] | None
    read_only: bool | None


class Outcome(TypedDict):
    budget: bool
    rate: bool | None
    raised: str | None
    reads: list[str]
    calls: list[CallJson]
    retained: list[DescriptorJson] | None
    warnings: list[str]


class Answer(TypedDict):
    outcomes: list[Outcome]


class CreateArgs(TypedDict):
    user_api_key_dict: UserAPIKeyAuth
    data: dict[str, str]
    rpm_limit_type: JsonValue
    tpm_limit_type: JsonValue
    model_has_failures: bool


def blank(call: str, token: str | None) -> CallJson:
    return CallJson(
        call=call, hook=None, model=None, token=token, rpm_limit_type=None, tpm_limit_type=None,
        model_has_failures=None, descriptors=None, parent_otel_span=None, read_only=None,
    )


def full(descriptor: RateLimitDescriptor) -> DescriptorJson:
    limits = descriptor["rate_limit"]
    canonical: LimitsJson | None = None
    if limits is not None:
        canonical = LimitsJson(
            requests_per_unit=limits.get("requests_per_unit"),
            tokens_per_unit=limits.get("tokens_per_unit"),
            max_parallel_requests=limits.get("max_parallel_requests"),
            window_size=limits.get("window_size"),
        )
    return DescriptorJson(key=descriptor["key"], value=descriptor["value"], rate_limit=canonical)


class ScriptedError(Exception):
    pass


class UnverifiedError(RateLimitUnverifiableError):
    def __init__(self, detail: str) -> None:
        super().__init__()
        self.detail = detail


class ScriptedCache(DualCache):
    def __init__(self, spend: dict[str, float], offline: list[str]) -> None:
        super().__init__()
        self.spend = spend
        self.offline = offline
        self.reads: list[str] = []

    @override
    async def async_get_cache(
        self,
        key: str,
        parent_otel_span: Span | None = None,
        local_only: bool = False,
        **kwargs: int,
    ) -> float | None:
        self.reads.append(key)
        if key in self.offline:
            raise ScriptedError("offline")
        return self.spend.get(key)


class ScriptedLimiter(CustomLogger):
    def __init__(self, script: RateScript, journal: list[CallJson]) -> None:
        self.script = script
        self.journal = journal
        self.retained: list[RateLimitDescriptor] | None = None
        off = script["missing"]
        team = partial(self.add, "team")
        project = partial(self.add, "project")
        self.should_rate_limit = None if "should_rate_limit" in off else self.check
        self._create_rate_limit_descriptors = (
            None if "_create_rate_limit_descriptors" in off else self.create
        )
        self._add_team_model_rate_limit_descriptor_from_metadata = (
            None if "_add_team_model_rate_limit_descriptor_from_metadata" in off else team
        )
        self._add_project_model_rate_limit_descriptor_from_metadata = (
            None if "_add_project_model_rate_limit_descriptor_from_metadata" in off else project
        )
        self.create_organization_rate_limit_descriptor = (
            None if "create_organization_rate_limit_descriptor" in off else self.organization
        )

    def record(self, stage: Stage, call: CallJson) -> None:
        self.journal.append(call)
        failure = self.script["failure"]
        if failure is None or failure["stage"] != stage:
            return
        if failure["kind"] == "unverifiable":
            raise UnverifiedError(failure["detail"])
        raise ScriptedError(failure["detail"])

    def create(self, **kwargs: Unpack[CreateArgs]) -> list[RateLimitDescriptor]:
        call = blank("create", kwargs["user_api_key_dict"].token)
        call["model"] = kwargs["data"]["model"]
        call["rpm_limit_type"] = kwargs["rpm_limit_type"]
        call["tpm_limit_type"] = kwargs["tpm_limit_type"]
        call["model_has_failures"] = kwargs["model_has_failures"]
        self.record("create", call)
        self.retained = list(self.script["base"])
        return self.retained

    def add(
        self,
        stage: Literal["team", "project"],
        *,
        user_api_key_dict: UserAPIKeyAuth,
        requested_model: str,
        descriptors: list[RateLimitDescriptor],
    ) -> None:
        call = blank(stage, user_api_key_dict.token)
        call["model"] = requested_model
        call["descriptors"] = [full(descriptor) for descriptor in descriptors]
        self.record(stage, call)
        descriptors.extend(self.script[stage])

    def organization(
        self, user_api_key_dict: UserAPIKeyAuth, requested_model: str
    ) -> list[RateLimitDescriptor]:
        call = blank("organization", user_api_key_dict.token)
        call["model"] = requested_model
        self.record("organization", call)
        return list(self.script["organization"])

    async def check(
        self,
        *,
        descriptors: Sequence[RateLimitDescriptor],
        parent_otel_span: Span | None,
        read_only: bool,
    ) -> RateLimitResponse:
        call = blank("check", None)
        call["descriptors"] = [full(descriptor) for descriptor in descriptors]
        call["parent_otel_span"] = None if parent_otel_span is None else "set"
        call["read_only"] = read_only
        self.record("check", call)
        return RateLimitResponse(overall_code=self.script["overall_code"], statuses=[])


class ScriptedHooks(ProxyLogging):
    def __init__(self, script: RateScript, journal: list[CallJson]) -> None:
        super().__init__(user_api_key_cache=proxy_server.user_api_key_cache)
        self.script = script
        self.journal = journal
        self.scripted = ScriptedLimiter(script, journal)

    @override
    def get_proxy_hook(self, hook: str) -> CustomLogger | None:
        call = blank("get_proxy_hook", None)
        call["hook"] = hook
        self.journal.append(call)
        if self.script["hook"] == "raises":
            raise ScriptedError("hooks offline")
        if self.script["hook"] == "none":
            return None
        return self.scripted


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(logging.WARNING)
        self.lines: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.lines.append(record.getMessage())


async def evaluate(summary_model: str, scenario: Scenario) -> Outcome:
    fields = scenario["principal"]
    principal = None if fields is None else PRINCIPALS.validate_python(fields)
    cache = ScriptedCache(scenario["spend"], scenario["offline"])
    proxy_server.model_max_budget_limiter.dual_cache = cache
    journal: list[CallJson] = []
    hooks = ScriptedHooks(scenario["rate"], journal)
    proxy_server.proxy_logging_obj = hooks
    budget = await CHECK_BUDGET(principal, summary_model)
    rate: bool | None = None
    raised: str | None = None
    try:
        rate = await CHECK_RATE(principal, summary_model)
    except ScriptedError as error:
        raised = str(error)
    kept = hooks.scripted.retained
    retained = None if kept is None else [full(descriptor) for descriptor in kept]
    return Outcome(
        budget=budget, rate=rate, raised=raised, reads=cache.reads, calls=journal,
        retained=retained, warnings=[],
    )


def run(input_value: Input) -> Answer:
    warnings = Warnings()
    logging.getLogger("LiteLLM").addHandler(warnings)
    outcomes: list[Outcome] = []
    for scenario in input_value["scenarios"]:
        outcome = asyncio.run(evaluate(input_value["summary_model"], scenario))
        outcome["warnings"] = warnings.lines
        warnings.lines = []
        outcomes.append(outcome)
    return Answer(outcomes=outcomes)
