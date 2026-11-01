from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import openai
from litellm.exceptions import AuthenticationError
from litellm.litellm_core_utils import exception_mapping_utils

# a working deployment sets this to keep exception_type()'s debug banner off stdout; the oracle
# wire protocol reads a single JSON line from the same stream
litellm.suppress_debug_info = True

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class RawErrorDetailInput(TypedDict):
    message: NotRequired[str | None]


class RawErrorBodyInput(TypedDict):
    type: NotRequired[str | None]
    param: NotRequired[str | None]
    code: NotRequired[str | None]
    message: NotRequired[str | None]
    error: NotRequired[RawErrorDetailInput | None]


class RawInput(TypedDict):
    kind: Literal["raw"]
    model: str
    custom_llm_provider: str
    error_str: str
    status_code: NotRequired[int | None]
    status_code_is_synthesized: NotRequired[bool]
    message: NotRequired[str | None]
    body: NotRequired[RawErrorBodyInput | None]
    exception_type: NotRequired[str]


class MappedInput(TypedDict):
    kind: Literal["mapped"]
    family: Literal["authentication"]
    message: str
    model: str
    provider: str


Input = RawInput | MappedInput


class _OriginalError(Exception):
    """Stands in for the SDK exception exception_type() normally introspects via getattr/hasattr."""

    def __init__(
        self,
        error_str: str,
        *,
        status_code: int | None = None,
        status_code_is_synthesized: bool = False,
        message: str | None = None,
        body: JsonValue = None,
    ) -> None:
        super().__init__(error_str)
        if status_code is not None:
            self.status_code = status_code
        self.status_code_is_synthesized = status_code_is_synthesized
        if message is not None:
            self.message = message
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


def _run_raw(input_value: RawInput) -> object:
    cause = _exception_class(input_value.get("exception_type") or "")(
        input_value["error_str"],
        status_code=input_value.get("status_code"),
        status_code_is_synthesized=input_value.get("status_code_is_synthesized", False),
        message=input_value.get("message"),
        body=cast(JsonValue, input_value.get("body")),
    )
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
