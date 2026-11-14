import asyncio
from hashlib import sha256
from typing import Final, Literal, NoReturn, Protocol

from litellm.exceptions import RateLimitError
from litellm.router import Router
from litellm.router_utils import clientside_credential_handler
from litellm.router_utils.client_initalization_utils import MaxParallelRequestsLimit
from litellm.types.llms.openai import AllMessageValues
from litellm.utils import calculate_max_parallel_requests
from pydantic import Field, JsonValue, TypeAdapter
from typing_extensions import override

from probes.deployment_identity import ID_SOURCE
from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false


class Completions(Protocol):
    async def acompletion(self, model: str, messages: list[AllMessageValues], *, mock_response: str) -> object: ...


class CacheReader(Protocol):
    def get_cache(self, key: str, *, local_only: bool = False) -> object | None: ...


class CredentialHandler(Protocol):
    @staticmethod
    def is_clientside_credential(request_kwargs: dict[str, object]) -> bool: ...

    @staticmethod
    def get_dynamic_litellm_params(
        litellm_params: dict[str, JsonValue], request_kwargs: dict[str, object]
    ) -> object: ...


CREDENTIAL_HANDLER: CredentialHandler = clientside_credential_handler
JSON_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
OPTIONAL_TEXT: Final = TypeAdapter(str | None)


class CalculateInput(Strict):
    max_parallel_requests: int | None = None
    rpm: int | None = None
    tpm: int | None = None
    default_max_parallel_requests: int | None = None


class Step(Strict):
    kind: Literal["acquire", "release", "enter_exit"]
    fail: bool = False


class LimitInput(Strict):
    max_parallel_requests: int
    model_id: str
    model_group: str
    steps: list[Step]


class CachedInput(Strict):
    model_id: str
    model_group: str
    max_parallel_requests: int | None = None
    rpm: int | None = None
    tpm: int | None = None
    default_max_parallel_requests: int | None = None
    lookups: int = 0


class RequestFields(Strict):
    api_key: str | None = None
    api_base: str | None = None
    base_url: str | None = None


class CredentialInput(Strict):
    model_group: str | None = None
    config: dict[str, JsonValue]
    request: RequestFields


class LimitError(Strict):
    status_code: int
    code: str | None
    error_type: str | None
    category: str | None
    rate_limit_type: str | None
    message: str


class LimitObservation(Strict):
    kind: str
    in_flight: int
    error: LimitError | None
    failed: bool


class CachedObservation(Strict):
    installed: bool
    present: bool
    reused: bool


class CredentialObservation(Strict):
    clientside: bool
    params: dict[str, JsonValue]
    api_key_sha256: str | None
    model_id: str | None


class Output(Strict):
    calculate: list[int | None]
    limits: list[list[LimitObservation]]
    cached: list[list[CachedObservation]]
    credentials: list[CredentialObservation]


def provider_failure() -> NoReturn:
    error = RuntimeError("the provider call failed")
    raise error


def error_fields(error: RateLimitError) -> LimitError:
    return LimitError(
        status_code=error.status_code,
        code=error.code,
        error_type=error.type,
        category=error.category,
        rate_limit_type=error.rate_limit_type,
        message=error.message,
    )


def limit_steps(spec: LimitInput) -> list[LimitObservation]:
    limit = MaxParallelRequestsLimit(
        max_parallel_requests=spec.max_parallel_requests,
        model_id=spec.model_id,
        model_group=spec.model_group,
    )
    observations: list[LimitObservation] = []
    for step in spec.steps:
        match step.kind:
            case "acquire":
                error: LimitError | None = None
                try:
                    limit.acquire()
                except RateLimitError as raised:
                    error = error_fields(raised)
                observations.append(
                    LimitObservation(kind="acquire", in_flight=limit.in_flight, error=error, failed=False)
                )
            case "release":
                limit.release()
                observations.append(
                    LimitObservation(kind="release", in_flight=limit.in_flight, error=None, failed=False)
                )
            case "enter_exit":
                error = None
                failed = False
                try:
                    with limit:
                        if step.fail:
                            provider_failure()
                except RateLimitError as raised:
                    error = error_fields(raised)
                except RuntimeError:
                    failed = True
                observations.append(
                    LimitObservation(kind="enter_exit", in_flight=limit.in_flight, error=error, failed=failed)
                )
    return observations


def model_entry(spec: CachedInput) -> dict[str, object]:
    given: dict[str, int | None] = {
        "max_parallel_requests": spec.max_parallel_requests,
        "rpm": spec.rpm,
        "tpm": spec.tpm,
    }
    params: dict[str, object] = {
        "model": "openai/gpt-4o",
        **{key: value for key, value in given.items() if value is not None},
    }
    return {
        "model_name": spec.model_group,
        "litellm_params": params,
        "model_info": {"id": spec.model_id},
    }


async def observe_lookups(spec: CachedInput, completions: Completions, cache: CacheReader) -> list[CachedObservation]:
    key = f"{spec.model_id}_max_parallel_requests_client"
    observations: list[CachedObservation] = []
    previous: object | None = None
    for _index in range(spec.lookups):
        installed = previous is None
        _response = await completions.acompletion(
            model=spec.model_group, messages=[{"role": "user", "content": "probe"}], mock_response="probe"
        )
        cached = cache.get_cache(key=key, local_only=True)
        observations.append(
            CachedObservation(
                installed=installed,
                present=cached is not None,
                reused=cached is not None and cached is previous,
            )
        )
        previous = cached
    return observations


def cached_lookups(spec: CachedInput) -> list[CachedObservation]:
    router = Router(
        model_list=[model_entry(spec)],
        default_max_parallel_requests=spec.default_max_parallel_requests,
    )
    return asyncio.run(observe_lookups(spec, router, router.cache))


def request_kwargs(fields: RequestFields) -> dict[str, object]:
    given = {"api_key": fields.api_key, "api_base": fields.api_base, "base_url": fields.base_url}
    return {key: value for key, value in given.items() if value is not None}


def credential_lookup(spec: CredentialInput) -> CredentialObservation:
    kwargs = request_kwargs(spec.request)
    clientside = CREDENTIAL_HANDLER.is_clientside_credential(kwargs)
    params = JSON_OBJECT.validate_python(CREDENTIAL_HANDLER.get_dynamic_litellm_params(dict(spec.config), kwargs))
    model_id = (
        ID_SOURCE.generate_model_id(spec.model_group, params)
        if clientside and spec.model_group is not None
        else None
    )
    api_key = OPTIONAL_TEXT.validate_python(params.pop("api_key", None))
    api_key_sha256 = None if api_key is None else sha256(api_key.encode()).hexdigest()
    return CredentialObservation(
        clientside=clientside, params=params, api_key_sha256=api_key_sha256, model_id=model_id
    )


class DeploymentClients(Operation):
    calculate: list[CalculateInput] = Field(default_factory=list)
    limits: list[LimitInput] = Field(default_factory=list)
    cached: list[CachedInput] = Field(default_factory=list)
    credentials: list[CredentialInput] = Field(default_factory=list)

    @override
    def run(self) -> Output:
        return Output(
            calculate=[
                calculate_max_parallel_requests(
                    max_parallel_requests=case.max_parallel_requests,
                    rpm=case.rpm,
                    tpm=case.tpm,
                    default_max_parallel_requests=case.default_max_parallel_requests,
                )
                for case in self.calculate
            ],
            limits=[limit_steps(spec) for spec in self.limits],
            cached=[cached_lookups(spec) for spec in self.cached],
            credentials=[credential_lookup(spec) for spec in self.credentials],
        )
