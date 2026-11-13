from collections.abc import Callable
from dataclasses import dataclass
from typing import Final, Protocol, TypeAlias, cast

import litellm
import litellm.utils as litellm_utils
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class RefusedError(Exception):
    pass


class Recorder:
    def __init__(self, *, fails: bool) -> None:
        self.fails = fails
        self.calls: list[Json] = []

    def completion(
        self,
        model: str,
        messages: list[dict[str, str]],
        api_key: str | None = None,
        max_tokens: int | None = None,
    ) -> ModelResponse:
        self.calls.append(
            {
                "kind": "completion",
                "model": model,
                "user_message": messages[0]["content"],
                "api_key": api_key,
                "max_tokens": max_tokens,
            }
        )
        if self.fails:
            raise RefusedError
        return ModelResponse()

    def retrieve(self, model: str) -> None:
        self.calls.append({"kind": "retrieve", "model": model})
        if self.fails:
            raise RefusedError


@dataclass(frozen=True)
class OpenAiStub:
    models: Recorder


@dataclass(frozen=True)
class Clock:
    time: Callable[[], float]


@dataclass(frozen=True)
class Dice:
    random: Callable[[], float]


class CompletionCall(Protocol):
    def __call__(
        self,
        model: str,
        messages: list[dict[str, str]],
        api_key: str | None = None,
        max_tokens: int | None = None,
    ) -> ModelResponse: ...


class ValidateEnvironmentCall(Protocol):
    def __call__(
        self,
        *,
        model: str | None,
        api_key: str | None,
        api_base: str | None,
        api_version: str | None,
    ) -> dict[str, Json]: ...


class Litellm(Protocol):
    completion: CompletionCall
    models_by_provider: dict[str, set[str]]
    open_ai_chat_completion_models: set[str]
    open_ai_text_completion_models: set[str]
    vercel_ai_gateway_models: set[str]
    edenai_models: set[str]
    datarobot_models: set[str]
    huggingface_models: set[str]
    together_ai_models: set[str]
    novita_models: set[str]
    nebius_models: set[str]
    wandb_models: set[str]


class Utils(Protocol):
    validate_environment: ValidateEnvironmentCall
    valid_model: Callable[[str], None]
    check_valid_key: Callable[[str, str], bool]
    openai: OpenAiStub
    time: Clock
    random: Dice
    INITIAL_RETRY_DELAY: float
    MAX_RETRY_DELAY: float
    JITTER: float


LITELLM: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L602-L718
    Litellm, litellm
)
UTILS: Final = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/utils.py#L7135-L7254
    Utils, litellm_utils
)
