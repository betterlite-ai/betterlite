from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

import openai
from litellm.litellm_core_utils import exception_mapping_utils as mapping

JsonValue: TypeAlias = None | bool | int | float | str | list["JsonValue"] | dict[str, "JsonValue"]


class AzureInnerErrorInput(TypedDict):
    code: NotRequired[str | None]


class AzureErrorDetailInput(TypedDict):
    type: NotRequired[str | None]
    param: NotRequired[str | None]
    code: NotRequired[str | None]
    message: NotRequired[str | None]
    inner_error: NotRequired[AzureInnerErrorInput | None]
    innererror: NotRequired[AzureInnerErrorInput | None]


class AzureErrorBodyInput(TypedDict):
    type: NotRequired[str | None]
    param: NotRequired[str | None]
    code: NotRequired[str | None]
    message: NotRequired[str | None]
    error: NotRequired[AzureErrorDetailInput | None]


class AzureInput(TypedDict):
    provider: Literal["azure"]
    model: str
    custom_llm_provider: str
    error_str: str
    exception_provider: str
    status_code: NotRequired[int | None]
    message: NotRequired[str | None]
    body: NotRequired[AzureErrorBodyInput | None]


class OpenrouterInput(TypedDict):
    provider: Literal["openrouter"]
    model: str
    custom_llm_provider: str
    error_str: str
    exception_provider: str
    status_code: NotRequired[int | None]
    status_code_is_synthesized: NotRequired[bool]


Input = AzureInput | OpenrouterInput


class _OriginalException:
    """Stands in for the SDK exception these mappers normally introspect via getattr/hasattr."""

    def __init__(
        self,
        error_str: str,
        *,
        status_code: int | None = None,
        body: JsonValue = None,
        message: str | None = None,
        status_code_is_synthesized: bool | None = None,
    ) -> None:
        if status_code is not None:
            self.status_code = status_code
        if body is not None:
            self.body = body
        if message is not None:
            self.message = message
        if status_code_is_synthesized is not None:
            self.status_code_is_synthesized = status_code_is_synthesized
        self._error_str = error_str

    def __str__(self) -> str:
        return self._error_str


class _ExceptionMapperArguments(TypedDict):
    model: str
    original_exception: object
    custom_llm_provider: str
    error_str: str
    exception_type: str
    exception_provider: str
    extra_information: str


class _ExceptionMapper(Protocol):
    def __call__(self, **kwargs: Unpack[_ExceptionMapperArguments]) -> None: ...


def _metadata(exc: openai.APIError) -> object:
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


def _run_azure(input_value: AzureInput) -> object:
    cause = _OriginalException(
        input_value["error_str"],
        status_code=input_value.get("status_code"),
        body=cast(JsonValue, input_value.get("body")),
        message=input_value.get("message"),
    )
    call = cast(_ExceptionMapper, mapping._map_azure_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=input_value["model"],
            original_exception=cause,
            custom_llm_provider=input_value["custom_llm_provider"],
            error_str=input_value["error_str"],
            exception_type="APIError",
            exception_provider=input_value["exception_provider"],
            extra_information="",
        )
    except openai.APIError as error:
        return _metadata(error)
    raise AssertionError("_map_azure_exception did not raise")


def _run_openrouter(input_value: OpenrouterInput) -> object:
    cause = _OriginalException(
        input_value["error_str"],
        status_code=input_value.get("status_code"),
        status_code_is_synthesized=input_value.get("status_code_is_synthesized", False),
    )
    call = cast(_ExceptionMapper, mapping._map_openrouter_exception)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    try:
        call(
            model=input_value["model"],
            original_exception=cause,
            custom_llm_provider=input_value["custom_llm_provider"],
            error_str=input_value["error_str"],
            exception_type="APIError",
            exception_provider=input_value["exception_provider"],
            extra_information="",
        )
    except openai.APIError as error:
        return _metadata(error)
    raise AssertionError("_map_openrouter_exception did not raise")


def run(input_value: Input) -> object:
    if input_value["provider"] == "azure":
        return _run_azure(input_value)
    return _run_openrouter(input_value)
