import asyncio
from typing import Final, Literal, NotRequired, Protocol, TypedDict
from unittest.mock import patch

import httpx
import litellm
from litellm.exceptions import (
    AuthenticationError,
    BadRequestError,
    InternalServerError,
    NotFoundError,
    PermissionDeniedError,
    RateLimitError,
    ServiceUnavailableError,
    Timeout,
)
from litellm.litellm_core_utils.streaming_handler import CustomStreamWrapper
from litellm.router import Router
from litellm.router_utils.add_retry_fallback_headers import get_hidden_params_dict
from litellm.types.llms.openai import AllMessageValues
from litellm.types.router import RetryPolicy as LiteLlmRetryPolicy
from litellm.types.utils import ModelResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from probes.protocol import Operation, ProbeError, Strict
from probes.retry_policy import ErrorInput, build_exception, rate_limit_with_headers

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

_GROUP: Final = "probe-group"
_MODEL: Final = "openai/gpt-4o"
_API_KEY: Final = "sk-probe"
_MESSAGES: Final[list[AllMessageValues]] = [{"role": "user", "content": "hi"}]

Outcome = Literal["answered", "failed", "cancelled", "timed_out"]


class Deployment(Strict):
    id: str
    num_retries: int | None = None


class Step(Strict):
    error: ErrorInput | None = None
    disconnect: bool = False
    retry_after: int | None = None


class Call(Strict):
    deployment: str
    attempted_retries: int
    max_retries: int
    breadcrumbs: int


class Headers(Strict):
    attempted_retries: int
    max_retries: int | None


class Failure(Strict):
    status: int
    code: str
    max_retries: int | None
    num_retries: int | None


class Output(Strict):
    outcome: Outcome
    calls: list[Call]
    backoffs: list[str]
    headers: Headers | None
    error: Failure | None


class ModelInfo(TypedDict):
    id: str


class Metadata(TypedDict):
    model_info: ModelInfo
    attempted_retries: int
    max_retries: int
    previous_models: NotRequired[list[JsonValue]]


class Arguments(TypedDict):
    metadata: Metadata


RetryHeaders = TypedDict(
    "RetryHeaders",
    {"x-litellm-attempted-retries": int, "x-litellm-max-retries": NotRequired[int]},
)


class Hidden(TypedDict):
    additional_headers: RetryHeaders


_ARGUMENTS: Final = TypeAdapter(Arguments)
_HIDDEN: Final = TypeAdapter(Hidden)

ProviderError = (
    AuthenticationError
    | NotFoundError
    | PermissionDeniedError
    | BadRequestError
    | Timeout
    | RateLimitError
    | ServiceUnavailableError
    | InternalServerError
)
_CAUGHT: Final = (
    AuthenticationError,
    NotFoundError,
    PermissionDeniedError,
    BadRequestError,
    Timeout,
    RateLimitError,
    ServiceUnavailableError,
    InternalServerError,
)


class Completions(Protocol):
    async def acompletion(
        self, *, model: str, messages: list[AllMessageValues], mock_response: str, stream: bool
    ) -> ModelResponse | CustomStreamWrapper: ...


_SDK: Final[Completions] = litellm


class RouterCompletions(Protocol):
    async def acompletion(
        self,
        *,
        model: str,
        messages: list[AllMessageValues],
        stream: bool,
        num_retries: int | None,
        mock_testing_rate_limit_error: bool,
    ) -> ModelResponse | CustomStreamWrapper: ...


# the provider each attempt reaches: it answers the scripted outcome and records what the router told it
class Provider:
    def __init__(self, steps: list[Step], *, stream: bool) -> None:
        self.steps: list[Step] = list(steps)
        self.stream: bool = stream
        self.calls: list[Call] = []
        self.backoffs: list[str] = []
        self.between_attempts: bool = False
        self.answer: Final = _SDK.acompletion

    async def acompletion(self, **kwargs: object) -> ModelResponse | CustomStreamWrapper:
        metadata = _ARGUMENTS.validate_python(kwargs)["metadata"]
        self.calls.append(
            Call(
                deployment=metadata["model_info"]["id"],
                attempted_retries=metadata["attempted_retries"],
                max_retries=metadata["max_retries"],
                breadcrumbs=len(metadata.get("previous_models", [])),
            )
        )
        self.between_attempts = False
        if not self.steps:
            raise ProbeError("invalid_request", "the router made more attempts than the case scripts")
        step = self.steps.pop(0)
        if step.disconnect:
            raise asyncio.CancelledError
        if step.error is not None:
            self.between_attempts = True
            if step.retry_after is not None:
                raise rate_limit_with_headers(httpx.Headers({"retry-after": str(step.retry_after)}))
            raise build_exception(step.error)
        return await self.answer(model=_MODEL, messages=_MESSAGES, mock_response="ok", stream=self.stream)

    # only the router's own backoff runs between a failed attempt and the next one
    async def sleep(self, delay: float) -> None:
        if self.between_attempts:
            self.backoffs.append(f"{delay:.6f}")


def model_list_entry(deployment: Deployment) -> dict[str, object]:
    params: dict[str, object] = {"model": _MODEL, "api_key": _API_KEY}
    if deployment.num_retries is not None:
        params["num_retries"] = deployment.num_retries
    return {"model_name": _GROUP, "litellm_params": params, "model_info": {"id": deployment.id}}


def headers(response: ModelResponse | CustomStreamWrapper) -> Headers:
    retry = _HIDDEN.validate_python(get_hidden_params_dict(response))["additional_headers"]
    return Headers(
        attempted_retries=retry["x-litellm-attempted-retries"],
        max_retries=retry.get("x-litellm-max-retries"),
    )


def failure(error: ProviderError) -> Failure:
    return Failure(
        status=error.status_code,
        code=str(error.status_code),
        max_retries=error.max_retries,
        num_retries=error.num_retries,
    )


class RoutingRetries(Operation):
    deployments: list[Deployment]
    num_retries: int
    request_num_retries: int | None = None
    retry_policy: LiteLlmRetryPolicy | None = None
    timeout: str
    stream: bool = False
    mock_testing_rate_limit_error: bool = False
    steps: list[Step]

    @override
    def run(self) -> Output:
        return asyncio.run(self.execute())

    async def execute(self) -> Output:
        router = Router(
            model_list=[model_list_entry(deployment) for deployment in self.deployments],
            num_retries=self.num_retries,
            retry_policy=self.retry_policy,
            timeout=float(self.timeout),
            disable_cooldowns=True,
            routing_strategy="simple-shuffle",
        )
        provider = Provider(self.steps, stream=self.stream)
        # the switch raises before any provider call, so the backoff after it is the router's too
        provider.between_attempts = self.mock_testing_rate_limit_error
        with patch.object(litellm, "acompletion", provider.acompletion), patch.object(asyncio, "sleep", provider.sleep):
            return await self.route(router, provider)

    # the router comes in through its protocol, so the call is typed without Router's untyped **kwargs
    async def route(self, router: RouterCompletions, provider: Provider) -> Output:
        outcome: Outcome = "answered"
        answered: Headers | None = None
        failed: Failure | None = None
        try:
            response = await router.acompletion(
                model=_GROUP,
                messages=_MESSAGES,
                stream=self.stream,
                num_retries=self.request_num_retries,
                mock_testing_rate_limit_error=self.mock_testing_rate_limit_error,
            )
            answered = headers(response)
        except asyncio.CancelledError:
            outcome = "cancelled"
        except _CAUGHT as error:
            outcome = "failed"
            failed = failure(error)
        return Output(
            outcome=outcome, calls=provider.calls, backoffs=provider.backoffs, headers=answered, error=failed
        )
