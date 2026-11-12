import io
import os
from contextlib import redirect_stdout
from dataclasses import dataclass, field
from functools import partial
from typing import Annotated, Final, Literal, TypeAlias, TypedDict

import httpx
import litellm.utils as litellm_utils
from litellm.exceptions import BadRequestError
from pydantic import ConfigDict, Field, TypeAdapter

from b5364.stubs import LITELLM, UTILS, Clock, Dice, Json, OpenAiStub, Recorder


@dataclass(kw_only=True)
class ModelSets:
    open_ai_chat_completion_models: list[str] = field(default_factory=list)
    open_ai_text_completion_models: list[str] = field(default_factory=list)
    vercel_ai_gateway_models: list[str] = field(default_factory=list)
    edenai_models: list[str] = field(default_factory=list)
    datarobot_models: list[str] = field(default_factory=list)
    huggingface_models: list[str] = field(default_factory=list)
    together_ai_models: list[str] = field(default_factory=list)
    novita_models: list[str] = field(default_factory=list)
    nebius_models: list[str] = field(default_factory=list)
    wandb_models: list[str] = field(default_factory=list)
    vertex_ai: list[str] = field(default_factory=list)


@dataclass(kw_only=True)
class EnvironmentCase:
    environment: list[str] = field(default_factory=list)
    model_sets: ModelSets = field(default_factory=ModelSets)
    model: str | None = None
    api_key: str | None = None
    api_base: str | None = None
    api_version: str | None = None


@dataclass(kw_only=True)
class ValidModelCase:
    model_sets: ModelSets = field(default_factory=ModelSets)
    model: str
    fails: bool


@dataclass(kw_only=True)
class KeyCase:
    model: str
    api_key: str
    fails: bool


@dataclass(kw_only=True)
class StatusCase:
    status_code: int


@dataclass(kw_only=True)
class HeaderCase:
    headers: dict[str, str] | None = None
    now: int


@dataclass(kw_only=True)
class BackoffCase:
    remaining_retries: int
    max_retries: int
    min_timeout: int = 0
    headers: dict[str, str] | None = None
    now: int
    random: float
    initial_retry_delay: float | None = None
    max_retry_delay: float | None = None
    jitter: float | None = None


@dataclass(kw_only=True)
class ValidateEnvironment:
    operation: Literal["validate_environment"]
    cases: list[EnvironmentCase]


@dataclass(kw_only=True)
class ValidModel:
    operation: Literal["valid_model"]
    cases: list[ValidModelCase]


@dataclass(kw_only=True)
class CheckValidKey:
    operation: Literal["check_valid_key"]
    cases: list[KeyCase]


@dataclass(kw_only=True)
class ShouldRetry:
    operation: Literal["should_retry"]
    cases: list[StatusCase]


@dataclass(kw_only=True)
class RetryAfterHeader:
    operation: Literal["retry_after_header"]
    cases: list[HeaderCase]


@dataclass(kw_only=True)
class CalculateRetryAfter:
    operation: Literal["calculate_retry_after"]
    cases: list[BackoffCase]


Operation: TypeAlias = (
    ValidateEnvironment
    | ValidModel
    | CheckValidKey
    | ShouldRetry
    | RetryAfterHeader
    | CalculateRetryAfter
)


class Environment(TypedDict):
    keys_in_environment: bool
    missing_keys: list[str]


class ModelError(TypedDict):
    status_code: int
    llm_provider: str
    model: str


class ModelAnswer(TypedDict):
    calls: list[Json]
    error: ModelError | None


class KeyAnswer(TypedDict):
    calls: list[Json]
    valid: bool


class RetryAnswer(TypedDict):
    retry: bool


class SecondsAnswer(TypedDict):
    seconds: int | None


class WaitAnswer(TypedDict):
    wait: float


Answer: TypeAlias = Environment | ModelAnswer | KeyAnswer | RetryAnswer | SecondsAnswer | WaitAnswer

OPERATION: Final = TypeAdapter(
    Annotated[Operation, Field(discriminator="operation")], config=ConfigDict(extra="forbid")
)
ENVIRONMENT: Final = TypeAdapter(Environment)


def install_sets(sets: ModelSets) -> None:
    LITELLM.open_ai_chat_completion_models = set(sets.open_ai_chat_completion_models)
    LITELLM.open_ai_text_completion_models = set(sets.open_ai_text_completion_models)
    LITELLM.vercel_ai_gateway_models = set(sets.vercel_ai_gateway_models)
    LITELLM.edenai_models = set(sets.edenai_models)
    LITELLM.datarobot_models = set(sets.datarobot_models)
    LITELLM.huggingface_models = set(sets.huggingface_models)
    LITELLM.together_ai_models = set(sets.together_ai_models)
    LITELLM.novita_models = set(sets.novita_models)
    LITELLM.nebius_models = set(sets.nebius_models)
    LITELLM.wandb_models = set(sets.wandb_models)
    LITELLM.models_by_provider["vertex_ai"] = set(sets.vertex_ai)


def environment_answer(operation: EnvironmentCase) -> Environment:
    install_sets(operation.model_sets)
    os.environ.clear()
    os.environ.update(dict.fromkeys(operation.environment, ""))
    # the provider lookup prints its provider list to stdout for a model with no provider
    with redirect_stdout(io.StringIO()):
        raw: dict[str, Json] = UTILS.validate_environment(
            model=operation.model,
            api_key=operation.api_key,
            api_base=operation.api_base,
            api_version=operation.api_version,
        )
    return ENVIRONMENT.validate_python(raw)


def valid_model_answer(operation: ValidModelCase) -> ModelAnswer:
    install_sets(operation.model_sets)
    recorder = Recorder(fails=operation.fails)
    LITELLM.completion = recorder.completion
    UTILS.openai = OpenAiStub(models=recorder)
    try:
        UTILS.valid_model(operation.model)
    except BadRequestError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L220-L233
        refused: ModelError = {
            "status_code": error.status_code,
            "llm_provider": error.llm_provider,  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L220-L233
            "model": error.model,  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L220-L233
        }
        return {"calls": recorder.calls, "error": refused}
    return {"calls": recorder.calls, "error": None}


def key_answer(operation: KeyCase) -> KeyAnswer:
    recorder = Recorder(fails=operation.fails)
    LITELLM.completion = recorder.completion
    valid = UTILS.check_valid_key(operation.model, operation.api_key)
    return {"calls": recorder.calls, "valid": valid}


def seconds_answer(operation: HeaderCase) -> SecondsAnswer:
    UTILS.time = Clock(time=partial(float, operation.now))
    headers = None if operation.headers is None else httpx.Headers(operation.headers)
    seconds = litellm_utils._get_retry_after_from_exception_header(headers)  # type: ignore[misc]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L7197-L7228
    return {"seconds": seconds}


def wait_answer(operation: BackoffCase) -> WaitAnswer:
    UTILS.time = Clock(time=partial(float, operation.now))
    UTILS.random = Dice(random=partial(float, operation.random))
    if operation.initial_retry_delay is not None:
        UTILS.INITIAL_RETRY_DELAY = operation.initial_retry_delay
    if operation.max_retry_delay is not None:
        UTILS.MAX_RETRY_DELAY = operation.max_retry_delay
    if operation.jitter is not None:
        UTILS.JITTER = operation.jitter
    headers = None if operation.headers is None else httpx.Headers(operation.headers)
    wait = litellm_utils._calculate_retry_after(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L7231-L7254
        remaining_retries=operation.remaining_retries,
        max_retries=operation.max_retries,
        response_headers=headers,
        min_timeout=operation.min_timeout,
    )
    return {"wait": float(wait)}


def run(input_value: Json) -> list[Answer]:
    operation = OPERATION.validate_python(input_value)
    match operation:
        case ValidateEnvironment():
            return [environment_answer(case) for case in operation.cases]
        case ValidModel():
            return [valid_model_answer(case) for case in operation.cases]
        case CheckValidKey():
            return [key_answer(case) for case in operation.cases]
        case ShouldRetry():
            return [
                {"retry": litellm_utils._should_retry(case.status_code)}  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L7168-L7194
                for case in operation.cases
            ]
        case RetryAfterHeader():
            return [seconds_answer(case) for case in operation.cases]
        case CalculateRetryAfter():
            return [wait_answer(case) for case in operation.cases]
