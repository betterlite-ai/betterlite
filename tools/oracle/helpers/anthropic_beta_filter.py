import logging
from typing import Final, Literal, Protocol, TypeAlias, TypedDict, cast

from litellm import anthropic_beta_headers_manager
from litellm.anthropic_beta_headers_manager import filter_and_transform_beta_headers
from litellm.llms.anthropic.pass_through.messages.transformation import (
    AnthropicMessagesConfig,
)
from pydantic import JsonValue as Json
from typing_extensions import override

UNKNOWN_PREFIX: Final = "Dropping unknown beta header '"
UNSUPPORTED_PREFIX: Final = "Dropping unsupported beta header '"
PROVIDER_MARK: Final = "' for provider '"


class MatrixFilterCall(TypedDict):
    function: Literal["matrix_filter"]
    provider: str
    tokens: list[str] | None
    headers: dict[str, Json] | None
    body: dict[str, Json] | None


class NativeInjectionCall(TypedDict):
    function: Literal["native_injection"]
    custom_llm_provider: str
    headers: dict[str, Json]
    optional_params: dict[str, Json]
    messages: list[Json]


Call: TypeAlias = MatrixFilterCall | NativeInjectionCall


class Input(TypedDict):
    matrix: dict[str, Json]
    calls: list[Call]


class RequestUpdateFn(Protocol):
    def __call__(
        self,
        headers: dict[str, Json],
        request_data: dict[str, Json],
        provider: str,
    ) -> tuple[dict[str, Json], dict[str, Json]]: ...


class NativeUpdater(Protocol):
    def __call__(
        self,
        headers: dict[str, Json],
        optional_params: dict[str, Json],
        custom_llm_provider: str,
        messages: list[Json],
    ) -> dict[str, Json]: ...


UPDATE_REQUEST: Final = cast(RequestUpdateFn, anthropic_beta_headers_manager.update_request_with_filtered_beta)  # noqa: E501  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L359
NATIVE_BETAS: Final = cast(NativeUpdater, AnthropicMessagesConfig._update_headers_with_anthropic_beta)  # noqa: E501, SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/anthropic/pass_through/messages/transformation.py#L611


class DropRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.DEBUG)
        self.drops: list[Json] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        message = record.getMessage()
        if message.startswith(UNKNOWN_PREFIX):
            token = message.removeprefix(UNKNOWN_PREFIX).split(PROVIDER_MARK)[0]
            self.drops.append({"token": token, "reason": "unknown"})
        if message.startswith(UNSUPPORTED_PREFIX):
            token = message.removeprefix(UNSUPPORTED_PREFIX).split(PROVIDER_MARK)[0]
            self.drops.append({"token": token, "reason": "unsupported"})


def run(input_value: Input) -> Json:
    anthropic_beta_headers_manager._BETA_HEADERS_CONFIG = input_value["matrix"]  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/anthropic_beta_headers_manager.py#L35
    logger = logging.getLogger("LiteLLM")
    recorder = DropRecorder()
    previous_handlers = logger.handlers
    previous_level = logger.level
    logger.handlers = [recorder]
    logger.setLevel(logging.DEBUG)
    try:
        answers = [answer(call, recorder) for call in input_value["calls"]]
    finally:
        logger.handlers = previous_handlers
        logger.setLevel(previous_level)
    return {"calls": answers}


def answer(call: Call, recorder: DropRecorder) -> Json:
    recorder.drops = []
    match call["function"]:
        case "matrix_filter":
            return matrix_filter_answer(call, recorder)
        case "native_injection":
            return native_injection_answer(call, recorder)


def matrix_filter_answer(call: MatrixFilterCall, recorder: DropRecorder) -> Json:
    tokens: list[Json] | None = None
    if call["tokens"] is not None:
        tokens = list(
            filter_and_transform_beta_headers(call["tokens"], call["provider"])
        )
    headers: dict[str, Json] | None = None
    body: dict[str, Json] | None = None
    if call["headers"] is not None or call["body"] is not None:
        given_headers: dict[str, Json] = (
            {} if call["headers"] is None else call["headers"]
        )
        given_body: dict[str, Json] = {} if call["body"] is None else call["body"]
        headers, body = UPDATE_REQUEST(given_headers, given_body, call["provider"])
    return {"headers": headers, "body": body, "tokens": tokens, "drops": recorder.drops}


def native_injection_answer(call: NativeInjectionCall, recorder: DropRecorder) -> Json:
    headers = NATIVE_BETAS(
        call["headers"],
        call["optional_params"],
        call["custom_llm_provider"],
        call["messages"],
    )
    return {"headers": headers, "body": None, "tokens": None, "drops": recorder.drops}
