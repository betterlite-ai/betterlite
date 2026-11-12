from collections.abc import Callable
from typing import Final, Literal, TypeAlias, TypeVar
from unittest.mock import patch

import httpx
import litellm
import litellm.utils
from litellm.exceptions import (
    AuthenticationError,
    BadRequestError,
    ContentPolicyViolationError,
    ContextWindowExceededError,
    InternalServerError,
    NotFoundError,
    PermissionDeniedError,
    RateLimitError,
    ServiceUnavailableError,
    Timeout,
)
from litellm.router import Router
from litellm.router_utils.get_retry_from_policy import reset_retry_policy
from litellm.types.router import RetryPolicy as LiteLlmRetryPolicy
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MESSAGE: Final = "probe failure"
_MODEL: Final = "openai/gpt-4o"
_PROVIDER: Final = "openai"
_PROBE_URL: Final = "https://probe.invalid"
_PINNED_INITIAL_RETRY_DELAY: Final = 0.5
_PINNED_MAX_RETRY_DELAY: Final = 8.0
_PINNED_JITTER: Final = 0.75

FailureClassName: TypeAlias = Literal[
    "authentication",
    "not_found",
    "permission_denied",
    "bad_request",
    "timeout",
    "rate_limit",
    "context_window_exceeded",
    "content_policy_violation",
    "service_unavailable",
    "internal_server",
    "unclassified",
]

FunctionName: TypeAlias = Literal["count", "reset", "eligibility", "sleep"]

T = TypeVar("T")


class ErrorInput(Strict):
    class_name: FailureClassName = Field(alias="class")
    status: int | None = None


class CountInput(Strict):
    error: ErrorInput
    model_group: str | None = None
    retry_policy: LiteLlmRetryPolicy | None = None
    model_group_retry_policy: dict[str, LiteLlmRetryPolicy] | None = None
    request_num_retries: int | None = None
    router_num_retries: int
    deployment_num_retries: int | None = None


class EligibilityStep(Strict):
    error: ErrorInput
    healthy_deployments: int
    all_deployments: int
    context_window_fallbacks: list[str] | None = None
    content_policy_fallbacks: list[str] | None = None
    regular_fallbacks: list[str] | None = None


class EligibilityInput(Strict):
    steps: list[EligibilityStep]


class SleepStep(Strict):
    healthy_deployments: int
    all_deployments: int
    retry_after_header: str | None = None
    remaining_retries: int
    max_retries: int
    min_timeout: int


class SleepInput(Strict):
    now_unix_millis: int
    steps: list[SleepStep]


class Output(Strict):
    num_retries: int | None = None
    policy_applies: bool | None = None
    policy: LiteLlmRetryPolicy | None = None
    decisions: list[bool] | None = None
    delays: list[str] | None = None
    retry_afters: list[str] | None = None


def _probe_request() -> httpx.Request:
    return httpx.Request("POST", _PROBE_URL)


def _optional_response(status: int | None) -> httpx.Response | None:
    if status is None:
        return None
    return httpx.Response(status, request=_probe_request())


def _required_response(status: int | None, fallback: int) -> httpx.Response:
    code = fallback if status is None else status
    return httpx.Response(code, request=_probe_request())


def _authentication(status: int | None) -> Exception:
    return AuthenticationError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_optional_response(status)
    )


def _not_found(status: int | None) -> Exception:
    return NotFoundError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_optional_response(status)
    )


def _permission_denied(status: int | None) -> Exception:
    return PermissionDeniedError(
        message=_MESSAGE,
        llm_provider=_PROVIDER,
        model=_MODEL,
        response=_required_response(status, 403),
    )


def _bad_request(status: int | None) -> Exception:
    return BadRequestError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_optional_response(status)
    )


def _timeout(status: int | None) -> Exception:
    return Timeout(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, exception_status_code=status
    )


def _rate_limit(status: int | None) -> Exception:
    return RateLimitError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_optional_response(status)
    )


def _context_window(status: int | None) -> Exception:
    return ContextWindowExceededError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_optional_response(status)
    )


def _content_policy(status: int | None) -> Exception:
    return ContentPolicyViolationError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=_optional_response(status)
    )


def _service_unavailable(status: int | None) -> Exception:
    return ServiceUnavailableError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_optional_response(status)
    )


def _internal_server(status: int | None) -> Exception:
    return InternalServerError(
        message=_MESSAGE, llm_provider=_PROVIDER, model=_MODEL, response=_optional_response(status)
    )


def _unclassified(_status: int | None) -> Exception:
    return Exception(_MESSAGE)


ExceptionFactory: TypeAlias = Callable[[int | None], Exception]

_FACTORIES: Final[dict[FailureClassName, ExceptionFactory]] = {
    "authentication": _authentication,
    "not_found": _not_found,
    "permission_denied": _permission_denied,
    "bad_request": _bad_request,
    "timeout": _timeout,
    "rate_limit": _rate_limit,
    "context_window_exceeded": _context_window,
    "content_policy_violation": _content_policy,
    "service_unavailable": _service_unavailable,
    "internal_server": _internal_server,
    "unclassified": _unclassified,
}


def build_exception(error: ErrorInput) -> Exception:
    factory = _FACTORIES[error.class_name]
    return factory(error.status)


def require(value: T | None, name: str) -> T:
    if value is None:
        raise ProbeError("invalid_request", f"{name} is required for this function")
    return value


def deployments(count: int) -> list[dict[str, str]]:
    return [{"model_name": _MODEL} for _ in range(count)]


def retry_after_headers(header: str | None) -> httpx.Headers | None:
    if header is None:
        return None
    return httpx.Headers({"retry-after": header})


def rate_limit_with_headers(headers: httpx.Headers | None) -> RateLimitError:
    response: httpx.Response | None = None
    if headers is not None:
        response = httpx.Response(429, headers=headers, request=_probe_request())
    return RateLimitError(
        message=_MESSAGE, model=_MODEL, llm_provider=_PROVIDER, response=response
    )


def retry_after_text(headers: httpx.Headers | None) -> str:
    value = litellm.utils._get_retry_after_from_exception_header(headers)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L7197-L7228
    return "invalid" if value == -1 else f"seconds:{value}"


class RetryPolicy(Operation):
    function: FunctionName
    count: CountInput | None = None
    eligibility: EligibilityInput | None = None
    sleep: SleepInput | None = None

    @override
    def run(self) -> Output:
        if self.function == "count":
            return self.count_result()
        if self.function == "reset":
            return Output(policy=reset_retry_policy())
        if self.function == "eligibility":
            return self.eligibility_result()
        return self.sleep_result()

    def count_result(self) -> Output:
        payload = require(self.count, "count")
        exception = build_exception(payload.error)
        request = payload.request_num_retries
        num_retries = request if request is not None else payload.router_num_retries
        if request is None and payload.deployment_num_retries is not None:
            num_retries = payload.deployment_num_retries
        group_policies: dict[str, LiteLlmRetryPolicy] = {}
        if payload.model_group_retry_policy is not None:
            group_policies = payload.model_group_retry_policy
        policy_applies = False
        if request != 0 and (
            payload.retry_policy is not None or payload.model_group_retry_policy is not None
        ):
            router = Router(
                model_list=[],
                retry_policy=payload.retry_policy,
                model_group_retry_policy=group_policies,
            )
            retries = router.get_num_retries_from_retry_policy(
                exception, model_group=payload.model_group
            )
            if retries is not None:
                num_retries = retries
                policy_applies = True
        return Output(num_retries=num_retries, policy_applies=policy_applies)

    def eligibility_result(self) -> Output:
        payload = require(self.eligibility, "eligibility")
        router = Router(model_list=[])
        decisions: list[bool] = []
        for step in payload.steps:
            exception = build_exception(step.error)
            try:
                _ = router.should_retry_this_error(  # pyright: ignore[reportUnknownMemberType]
                    error=exception,
                    healthy_deployments=deployments(step.healthy_deployments),
                    all_deployments=deployments(step.all_deployments),
                    context_window_fallbacks=step.context_window_fallbacks,
                    content_policy_fallbacks=step.content_policy_fallbacks,
                    regular_fallbacks=step.regular_fallbacks,
                )
            except Exception as raised:
                if raised is not exception:
                    raise
                decisions.append(False)
            else:
                decisions.append(True)
        return Output(decisions=decisions)

    def sleep_result(self) -> Output:
        payload = require(self.sleep, "sleep")
        delays: list[str] = []
        retry_afters: list[str] = []
        with (
            patch.object(litellm.utils, "INITIAL_RETRY_DELAY", _PINNED_INITIAL_RETRY_DELAY),
            patch.object(litellm.utils, "MAX_RETRY_DELAY", _PINNED_MAX_RETRY_DELAY),
            patch.object(litellm.utils, "JITTER", _PINNED_JITTER),
        ):
            for step in payload.steps:
                headers = retry_after_headers(step.retry_after_header)
                exception = rate_limit_with_headers(headers)
                router = Router(model_list=[], retry_after=step.min_timeout)
                retry_afters.append(retry_after_text(headers))
                timeout = router._time_to_sleep_before_retry(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/router.py#L8010-L8053
                    e=exception,
                    remaining_retries=step.remaining_retries,
                    num_retries=step.max_retries,
                    healthy_deployments=deployments(step.healthy_deployments),
                    all_deployments=deployments(step.all_deployments),
                )
                delays.append(f"{timeout:.6f}")
        return Output(delays=delays, retry_afters=retry_afters)
