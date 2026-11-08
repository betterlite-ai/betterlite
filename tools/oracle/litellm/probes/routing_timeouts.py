from typing import Annotated, Final, Literal

import httpx
import litellm
from litellm.exceptions import Timeout
from litellm.litellm_core_utils.completion_timeout import CompletionTimeout
from litellm.litellm_core_utils.request_timeout_resolver import get_configured_request_timeout
from litellm.router import Router
from litellm.utils import supports_httpx_timeout
from pydantic import Field
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_MODEL: Final = "openai/gpt-4o"


class GlobalInput(Strict):
    seconds: str
    explicitly_set: bool


class ConfiguredInput(Strict):
    seconds: str
    explicitly_set: bool


class SecondsTimeout(Strict):
    kind: Literal["seconds"]
    seconds: str


class PhasesTimeout(Strict):
    kind: Literal["phases"]
    connect: str | None = None
    read: str | None = None
    write: str | None = None
    pool: str | None = None


TimeoutInput = Annotated[SecondsTimeout | PhasesTimeout, Field(discriminator="kind")]


class CompletionInput(Strict):
    model_timeout: TimeoutInput | None = None
    timeout: str | None = None
    request_timeout: str | None = None
    provider: str
    global_timeout: GlobalInput


class KwargsInput(Strict):
    stream: bool = False
    timeout: str | None = None
    request_timeout: str | None = None
    stream_timeout: str | None = None


class DeploymentInput(Strict):
    timeout: str | None = None
    request_timeout: str | None = None
    stream_timeout: str | None = None


class RouterInput(Strict):
    timeout: str | None = None
    stream_timeout: str | None = None
    default_timeout: str | None = None
    default_stream_timeout: str | None = None
    global_timeout: GlobalInput
    kwargs: KwargsInput = Field(default_factory=KwargsInput)
    deployment: DeploymentInput = Field(default_factory=DeploymentInput)
    elapsed_millis: int | None = None
    cancelled: bool = False


class SecondsOutput(Strict):
    kind: Literal["seconds"]
    seconds: str


class PhasesOutput(Strict):
    kind: Literal["phases"]
    connect: str | None
    read: str | None
    write: str | None
    pool: str | None


TimeoutOutput = Annotated[SecondsOutput | PhasesOutput, Field(discriminator="kind")]


class CompletionResult(Strict):
    resolved: TimeoutOutput
    transport: TimeoutOutput


class RouterResult(Strict):
    stream: TimeoutOutput | None
    non_stream: TimeoutOutput | None
    attempt: TimeoutOutput | None
    deadline_millis: int | None
    attempt_started: bool
    expired_status: int | None
    expired_code: str | None


class Output(Strict):
    configured: str | None = None
    resolved: TimeoutOutput | None = None
    transport: TimeoutOutput | None = None
    stream: TimeoutOutput | None = None
    non_stream: TimeoutOutput | None = None
    attempt: TimeoutOutput | None = None
    deadline_millis: int | None = None
    attempt_started: bool | None = None
    expired_status: int | None = None
    expired_code: str | None = None
    cancelled: bool | None = None


def parse_seconds(text: str, name: str) -> float:
    try:
        return float(text)
    except ValueError as error:
        raise ProbeError("invalid_request", f"{name} is not a number: {text}") from error


def optional_seconds(text: str | None, name: str) -> float | None:
    if text is None:
        return None
    return parse_seconds(text, name)


def format_seconds(value: float | None) -> str | None:
    if value is None:
        return None
    return f"{value:.6f}"


def set_global(global_timeout: GlobalInput) -> None:
    litellm.request_timeout = parse_seconds(global_timeout.seconds, "request_timeout")  # pyright: ignore[reportPrivateImportUsage]
    litellm.request_timeout_explicitly_set = global_timeout.explicitly_set


def timeout_input(value: SecondsTimeout | PhasesTimeout) -> float | httpx.Timeout:
    if isinstance(value, SecondsTimeout):
        return parse_seconds(value.seconds, "model_timeout")
    return httpx.Timeout(
        connect=optional_seconds(value.connect, "connect"),
        read=optional_seconds(value.read, "read"),
        write=optional_seconds(value.write, "write"),
        pool=optional_seconds(value.pool, "pool"),
    )


def timeout_output(value: float | httpx.Timeout) -> SecondsOutput | PhasesOutput:
    if isinstance(value, httpx.Timeout):
        return PhasesOutput(
            kind="phases",
            connect=format_seconds(value.connect),
            read=format_seconds(value.read),
            write=format_seconds(value.write),
            pool=format_seconds(value.pool),
        )
    return SecondsOutput(kind="seconds", seconds=f"{value:.6f}")


def no_phases(_provider: str) -> bool:
    return False


def completion_result(completion: CompletionInput) -> CompletionResult:
    set_global(completion.global_timeout)
    model_timeout = None if completion.model_timeout is None else timeout_input(completion.model_timeout)
    kwargs: dict[str, object] = {}
    timeout = optional_seconds(completion.timeout, "kwargs timeout")
    if timeout is not None:
        kwargs["timeout"] = timeout
    request_timeout = optional_seconds(completion.request_timeout, "kwargs request_timeout")
    if request_timeout is not None:
        kwargs["request_timeout"] = request_timeout
    configured = get_configured_request_timeout()
    return CompletionResult(
        resolved=timeout_output(
            CompletionTimeout.resolve(  # pyright: ignore[reportUnknownMemberType]
                model_timeout,
                kwargs,
                completion.provider,
                global_timeout=configured,
                supports_httpx_timeout=supports_httpx_timeout,
            )
        ),
        transport=timeout_output(
            CompletionTimeout.resolve(  # pyright: ignore[reportUnknownMemberType]
                model_timeout,
                kwargs,
                completion.provider,
                global_timeout=configured,
                supports_httpx_timeout=no_phases,
            )
        ),
    )


def router_defaults(router: RouterInput) -> dict[str, float]:
    defaults: dict[str, float] = {}
    default_timeout = optional_seconds(router.default_timeout, "default timeout")
    if default_timeout is not None:
        defaults["timeout"] = default_timeout
    default_stream_timeout = optional_seconds(router.default_stream_timeout, "default stream timeout")
    if default_stream_timeout is not None:
        defaults["stream_timeout"] = default_stream_timeout
    return defaults


def router_kwargs(router: RouterInput) -> dict[str, object]:
    kwargs: dict[str, object] = {"stream": router.kwargs.stream}
    timeout = optional_seconds(router.kwargs.timeout, "kwargs timeout")
    if timeout is not None:
        kwargs["timeout"] = timeout
    request_timeout = optional_seconds(router.kwargs.request_timeout, "kwargs request_timeout")
    if request_timeout is not None:
        kwargs["request_timeout"] = request_timeout
    stream_timeout = optional_seconds(router.kwargs.stream_timeout, "kwargs stream_timeout")
    if stream_timeout is not None:
        kwargs["stream_timeout"] = stream_timeout
    return kwargs


def deployment_params(router: RouterInput) -> dict[str, object]:
    params: dict[str, object] = {}
    timeout = optional_seconds(router.deployment.timeout, "litellm_params timeout")
    if timeout is not None:
        params["timeout"] = timeout
    request_timeout = optional_seconds(router.deployment.request_timeout, "litellm_params request_timeout")
    if request_timeout is not None:
        params["request_timeout"] = request_timeout
    stream_timeout = optional_seconds(router.deployment.stream_timeout, "litellm_params stream_timeout")
    if stream_timeout is not None:
        params["stream_timeout"] = stream_timeout
    return params


def build_router(router: RouterInput) -> Router:
    set_global(router.global_timeout)
    model_list: list[dict[str, object]] = [
        {
            "model_name": "probe",
            "litellm_params": {"model": _MODEL, "mock_response": "probe"},
        }
    ]
    return Router(
        model_list=model_list,
        timeout=optional_seconds(router.timeout, "router timeout"),
        stream_timeout=optional_seconds(router.stream_timeout, "router stream timeout"),
        default_litellm_params=router_defaults(router),
    )


def expiry_status(router: RouterInput, attempt: float | None) -> tuple[int | None, str | None]:
    if router.elapsed_millis is None or attempt is None:
        return None, None
    if router.elapsed_millis >= attempt * 1000:
        status = Timeout("probe", model=_MODEL, llm_provider="openai").status_code
        return status, str(status)
    return None, None


def router_result(settings: RouterInput) -> RouterResult:
    router = build_router(settings)
    kwargs = router_kwargs(settings)
    params = deployment_params(settings)
    attempt = router._get_timeout(kwargs, params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    stream = router._get_stream_timeout(kwargs, params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    non_stream = router._get_non_stream_timeout(kwargs, params)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    whole = router._get_timeout(kwargs, {})  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    expired_status, expired_code = expiry_status(settings, attempt)
    return RouterResult(
        stream=None if stream is None else timeout_output(stream),
        non_stream=None if non_stream is None else timeout_output(non_stream),
        attempt=None if attempt is None else timeout_output(attempt),
        deadline_millis=None if whole is None else round(float(whole) * 1000),
        attempt_started=True,
        expired_status=expired_status,
        expired_code=expired_code,
    )


class RoutingTimeouts(Operation):
    configured: ConfiguredInput | None = None
    completion: CompletionInput | None = None
    router: RouterInput | None = None

    @override
    def run(self) -> Output:
        configured: str | None = None
        if self.configured is not None:
            set_global(
                GlobalInput(
                    seconds=self.configured.seconds,
                    explicitly_set=self.configured.explicitly_set,
                )
            )
            configured = format_seconds(get_configured_request_timeout())
        completion = None if self.completion is None else completion_result(self.completion)
        router = None if self.router is None else router_result(self.router)
        return Output(
            configured=configured,
            resolved=None if completion is None else completion.resolved,
            transport=None if completion is None else completion.transport,
            stream=None if router is None else router.stream,
            non_stream=None if router is None else router.non_stream,
            attempt=None if router is None else router.attempt,
            deadline_millis=None if router is None else router.deadline_millis,
            attempt_started=None if router is None else router.attempt_started,
            expired_status=None if router is None else router.expired_status,
            expired_code=None if router is None else router.expired_code,
        )
