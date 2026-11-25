from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import openai
from litellm.exceptions import AuthenticationError
from litellm.litellm_core_utils import exception_mapping_utils
from litellm.llms.base_llm.chat.transformation import BaseLLMException

# a working deployment sets this to keep exception_type()'s debug banner off stdout; the oracle
# wire protocol reads a single JSON line from the same stream
litellm.suppress_debug_info = True

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class RawInput(TypedDict):
    kind: Literal["raw"]
    model: str
    custom_llm_provider: str
    error_str: str
    status_code: NotRequired[int | None]
    status_code_is_synthesized: NotRequired[bool]
    message: NotRequired[str | None]
    body: NotRequired[dict[str, JsonValue] | None]
    exception_type: NotRequired[str]


class MappedInput(TypedDict):
    kind: Literal["mapped"]
    family: Literal["authentication"]
    message: str
    model: str
    provider: str


Input = RawInput | MappedInput


class _OriginalError(Exception):
    """Stands in for an SDK exception that carries no status code."""

    def __init__(
        self,
        error_str: str,
        *,
        message: str | None = None,
        body: dict[str, JsonValue] | None = None,
    ) -> None:
        super().__init__(error_str)
        # a real SDK exception always carries .message; the native elif chain reads it unguarded
        self.message = message if message is not None else error_str
        if body is not None:
            self.body = body
        self._error_str = error_str

    def __str__(self) -> str:
        return self._error_str


# type(original_exception).__name__ is what exception_type() reads as its own "exception_type"
# variable; a distinct subclass name lets a raw case stand in for replicate's ModelError or
# cohere's CohereConnectionError without instantiating either SDK's real exception class
def _exception_class(name: str) -> type[_OriginalError]:
    if not name:
        return _OriginalError
    return type(name, (_OriginalError,), {})


# a status-bearing input is a provider HTTP error, which LiteLLM models with BaseLLMException
def _status_exception_class(name: str) -> type[BaseLLMException]:
    if not name:
        return BaseLLMException
    return type(name, (BaseLLMException,), {})


class _ExceptionTypeArguments(TypedDict):
    model: str
    original_exception: object
    custom_llm_provider: str


class _ExceptionTypeFn(Protocol):
    def __call__(self, **kwargs: Unpack[_ExceptionTypeArguments]) -> object: ...


_exception_type = cast(_ExceptionTypeFn, exception_mapping_utils.exception_type)


def _metadata(exc: object) -> object:
    status_code = getattr(exc, "status_code", 500)
    return {
        "status_code": status_code,
        "model": getattr(exc, "model", None),
        "provider": getattr(exc, "llm_provider", None),
        "type": getattr(exc, "type", "None"),
        "param": getattr(exc, "param", "None"),
        "code": str(status_code),
        "retry": {
            "max_retries": getattr(exc, "max_retries", None),
            "num_retries": getattr(exc, "num_retries", None),
        },
        "rate_limit_category": getattr(exc, "category", None),
        "rate_limit_dimension": getattr(exc, "rate_limit_type", None),
        "allowed_response_headers": {},
        "openai_code": getattr(exc, "code", None),
    }


def _dispatch(model: str, original_exception: object, custom_llm_provider: str) -> object:
    # exception_type() returns the exception unchanged for the preamble's mapped passthrough case,
    # and otherwise raises the mapped exception instead of returning it.
    try:
        return _exception_type(
            model=model,
            original_exception=original_exception,
            custom_llm_provider=custom_llm_provider,
        )
    except openai.APIError as mapped:
        return mapped


def _raw_exception(input_value: RawInput) -> Exception:
    name = input_value.get("exception_type") or ""
    status_code = input_value.get("status_code")
    message = input_value.get("message")
    if message is None:
        message = input_value["error_str"]
    if status_code is None:
        return _exception_class(name)(
            input_value["error_str"],
            message=message,
            body=input_value.get("body"),
        )
    return _status_exception_class(name)(
        status_code=status_code,
        message=message,
        status_code_is_synthesized=input_value.get("status_code_is_synthesized", False),
        body=input_value.get("body"),
    )


def _run_raw(input_value: RawInput) -> object:
    cause = _raw_exception(input_value)
    mapped = _dispatch(input_value["model"], cause, input_value["custom_llm_provider"])
    return _metadata(mapped)


def _run_mapped(input_value: MappedInput) -> object:
    original = AuthenticationError(
        message=input_value["message"],
        llm_provider=input_value["provider"],
        model=input_value["model"],
    )
    mapped = _dispatch(input_value["model"], original, input_value["provider"])
    return _metadata(mapped)


def run(input_value: Input) -> object:
    if input_value["kind"] == "raw":
        return _run_raw(input_value)
    return _run_mapped(input_value)
