from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import litellm
import openai
from litellm.exceptions import AuthenticationError, GuardrailRaisedException
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


class AuthenticationMappedInput(TypedDict):
    kind: Literal["mapped"]
    family: Literal["authentication"]
    message: str
    model: str
    provider: str
    max_retries: NotRequired[int | None]
    num_retries: NotRequired[int | None]


class GuardrailMappedInput(TypedDict):
    kind: Literal["mapped"]
    family: Literal["guardrail_raised"]
    guardrail_name: NotRequired[str | None]
    message: str
    status_code: int
    blocked_content: bool


Input = RawInput | AuthenticationMappedInput | GuardrailMappedInput


class _OriginalException:
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
    # exception_type() returns the exception unchanged for the preamble's two passthrough cases,
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
    cause = _OriginalException(
        input_value["error_str"],
        status_code=input_value.get("status_code"),
        status_code_is_synthesized=input_value.get("status_code_is_synthesized", False),
        message=input_value.get("message"),
        body=cast(JsonValue, input_value.get("body")),
    )
    mapped = _dispatch(input_value["model"], cause, input_value["custom_llm_provider"])
    return _metadata(mapped)


def _run_mapped_authentication(input_value: AuthenticationMappedInput) -> object:
    original = AuthenticationError(
        message=input_value["message"],
        llm_provider=input_value["provider"],
        model=input_value["model"],
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    mapped = _dispatch(input_value["model"], original, input_value["provider"])
    return _metadata(mapped)


def _run_mapped_guardrail(input_value: GuardrailMappedInput) -> object:
    original = GuardrailRaisedException(
        guardrail_name=input_value.get("guardrail_name"),
        message=input_value["message"],
        status_code=input_value["status_code"],
        blocked_content=input_value["blocked_content"],
    )
    mapped = _dispatch("", original, "")
    return _metadata(mapped)


def run(input_value: Input) -> object:
    if input_value["kind"] == "raw":
        return _run_raw(input_value)
    if input_value["family"] == "authentication":
        return _run_mapped_authentication(input_value)
    return _run_mapped_guardrail(input_value)
