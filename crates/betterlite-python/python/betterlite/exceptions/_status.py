from collections.abc import Iterable, Sequence
from dataclasses import dataclass
from typing import Final, Required, TypeAlias, Unpack

from pydantic import JsonValue
from typing_extensions import ReadOnly, TypedDict, override

from betterlite.exceptions import _openai_base as base
from betterlite.exceptions._http import Request, Response
from betterlite.exceptions._text import RetryText

_MINIMAL_ERROR_RESPONSE: Final[Response] = Response(
    status_code=400,
    request=Request(method="GET", url="https://litellm.ai"),
)
_UNKNOWN_PROVIDER_MESSAGE: Final[str] = (
    "Unmapped LLM provider for this endpoint. You passed model={model}, "
    "custom_llm_provider={custom_llm_provider}. Check supported provider and route: "
    "https://docs.litellm.ai/docs/providers"
)
# ast-grep-ignore: no-long-names
VECTOR_STORE_SEARCH_FAILED_CODE: Final[str] = "vector_store_search_failed"
_VECTOR_STORE_SEARCH_BODY: Final[dict[str, JsonValue]] = {
    "type": "invalid_request_error",
    "code": VECTOR_STORE_SEARCH_FAILED_CODE,  # ast-grep-ignore: no-long-names
}
class VectorStoreSearchFailure(TypedDict):
    vector_store_id: ReadOnly[str]
    custom_llm_provider: ReadOnly[str | None]
    error: ReadOnly[str]
class CommonErrorOptions(TypedDict, total=False):
    message: Required[str]
    model: Required[str | None]
    llm_provider: Required[str | None]
    litellm_debug_info: str | None
    max_retries: int | None
    num_retries: int | None
class BadRequestOptions(CommonErrorOptions, total=False):
    response: Response | None
    body: JsonValue
class ContentPolicyViolationOptions(CommonErrorOptions, total=False):
    response: Response | None
    provider_specific_fields: dict[str, JsonValue] | None
    body: JsonValue
class ImageFetchOptions(TypedDict, total=False):
    message: Required[str]
    model: str | None
    llm_provider: str | None
    response: Response | None
    litellm_debug_info: str | None
    max_retries: int | None
    num_retries: int | None
    body: JsonValue
class UnsupportedParamsOptions(TypedDict, total=False):
    message: Required[str]
    model: str | None
    llm_provider: str | None
    status_code: int
    response: Response | None
    litellm_debug_info: str | None
    max_retries: int | None
    num_retries: int | None
_BAD_REQUEST_KEYS: Final[frozenset[str]] = (
    BadRequestOptions.__optional_keys__ | BadRequestOptions.__required_keys__
)
_UNSUPPORTED_PARAMS_KEYS: Final[frozenset[str]] = (
    UnsupportedParamsOptions.__optional_keys__ | UnsupportedParamsOptions.__required_keys__
)
_IMAGE_FETCH_KEYS: Final[frozenset[str]] = (
    ImageFetchOptions.__optional_keys__ | ImageFetchOptions.__required_keys__
)
def check_options(
    names: Iterable[str], allowed: frozenset[str], required: frozenset[str], call: str,
) -> None:
    present = frozenset(names)
    if missing := [name for name in required if name not in present]:
        raise TypeError(f"{call}() missing required arguments: {', '.join(missing)}")
    if unknown := [name for name in names if name not in allowed]:
        raise TypeError(f"{call}() got unexpected keyword arguments: {', '.join(unknown)}")
def mock_response(status_code: int) -> Response:
    request = Request(method="GET", url="https://litellm.ai")
    return Response(status_code=status_code, request=request)
@dataclass(eq=False, repr=False)
class AuthenticationError(RetryText, base.AuthenticationError):
    message: str
    llm_provider: str
    model: str
    response: Response | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        self.status_code = 401
        self.message = f"litellm.AuthenticationError: {self.message}"
        response = mock_response(401) if self.response is None else self.response
        self.response = response
        super().__init__(self.message, response=response, body=None)
@dataclass(eq=False, repr=False)
class NotFoundError(RetryText, base.NotFoundError):
    message: str
    model: str
    llm_provider: str
    response: Response | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        self.status_code = 404
        self.message = f"litellm.NotFoundError: {self.message}"
        response = mock_response(404) if self.response is None else self.response
        self.response = response
        super().__init__(self.message, response=response, body=None)
class BadRequestError(RetryText, base.BadRequestError):
    model: str | None
    llm_provider: str | None
    litellm_debug_info: str | None
    status_code: int
    def __init__(self, **options: Unpack[BadRequestOptions]) -> None:
        check_options(
            options, _BAD_REQUEST_KEYS, BadRequestOptions.__required_keys__, "BadRequestError",
        )
        apply_common_fields(self, 400, "BadRequestError", options)
        response = options.get("response")
        if response is None or not response.has_request():
            response = _MINIMAL_ERROR_RESPONSE
        super().__init__(self.message, response=response, body=options.get("body"))
class ImageFetchError(BadRequestError):
    def __init__(self, **options: Unpack[ImageFetchOptions]) -> None:
        check_options(
            options, _IMAGE_FETCH_KEYS, ImageFetchOptions.__required_keys__, "ImageFetchError",
        )
        super().__init__(
            message=options["message"],
            model=options.get("model"),
            llm_provider=options.get("llm_provider"),
            response=options.get("response"),
            litellm_debug_info=options.get("litellm_debug_info"),
            max_retries=options.get("max_retries"),
            num_retries=options.get("num_retries"),
            body=options.get("body"),
        )
def apply_common_fields(
    error: BadRequestError,
    status_code: int,
    name: str,
    options: CommonErrorOptions | ImageFetchOptions | UnsupportedParamsOptions,
) -> None:
    error.status_code = status_code
    error.message = f"litellm.{name}: {options['message']}"
    error.model = options.get("model")
    error.llm_provider = options.get("llm_provider")
    error.litellm_debug_info = options.get("litellm_debug_info")
    error.max_retries = options.get("max_retries")
    error.num_retries = options.get("num_retries")
class VectorStoreSearchError(BadRequestError):
    failures: tuple[VectorStoreSearchFailure, ...]
    def __init__(
        self,
        failures: Sequence[VectorStoreSearchFailure],
        model: str | None = None,
        llm_provider: str | None = None,
    ) -> None:
        self.failures = tuple(failures)
        detail = "; ".join(
            f"{failure['vector_store_id']}: {failure['error']}" for failure in self.failures
        )
        super().__init__(
            message=(
                "The request could not be grounded in every configured vector store. "
                f"{len(self.failures)} vector store search(es) failed: {detail}"
            ),
            model=model,
            llm_provider=llm_provider,
            body=_VECTOR_STORE_SEARCH_BODY,
        )
@dataclass(eq=False, repr=False)
class UnprocessableEntityError(RetryText, base.UnprocessableEntityError):
    message: str
    model: str
    llm_provider: str
    response: Response | None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        self.status_code = 422
        self.message = f"litellm.UnprocessableEntityError: {self.message}"
        response = self.response
        if response is None:
            raise TypeError("UnprocessableEntityError() response must be a Response")
        super().__init__(self.message, response=response, body=None)
@dataclass(eq=False, repr=False)
class PermissionDeniedError(RetryText, base.PermissionDeniedError):
    message: str
    llm_provider: str
    model: str
    response: Response | None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        self.status_code = 403
        self.message = f"litellm.PermissionDeniedError: {self.message}"
        response = self.response
        if response is None:
            raise TypeError("PermissionDeniedError() response must be a Response")
        super().__init__(self.message, response=response, body=None)
class ContextWindowExceededError(BadRequestError):
    def __init__(
        self,
        message: str,
        model: str,
        llm_provider: str,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
    ) -> None:
        super().__init__(
            message=message,
            model=model,
            llm_provider=llm_provider,
            response=response,
            litellm_debug_info=litellm_debug_info,
        )
        self.message = f"litellm.ContextWindowExceededError: {self.message}"
class RejectedRequestError(BadRequestError):
    def __init__(
        self,
        message: str,
        model: str,
        llm_provider: str,
        request_data: dict[str, JsonValue],
        litellm_debug_info: str | None = None,
    ) -> None:
        self.message = f"litellm.RejectedRequestError: {message}"
        self.request_data = request_data
        request = Request(method="POST", url="https://api.openai.com/v1")
        response = Response(status_code=400, request=request)
        super().__init__(
            message=self.message,
            model=model,
            llm_provider=llm_provider,
            response=response,
            litellm_debug_info=litellm_debug_info,
        )
@dataclass(eq=False, repr=False)
class ContentPolicyViolationError(BadRequestError):
    message: str
    model: str | None
    llm_provider: str | None
    response: Response | None = None
    litellm_debug_info: str | None = None
    provider_specific_fields: dict[str, JsonValue] | None = None
    body: JsonValue = None
    def __post_init__(self) -> None:
        self.message = f"litellm.ContentPolicyViolationError: {self.message}"
        super().__init__(
            message=self.message,
            model=self.model,
            llm_provider=self.llm_provider,
            response=self.response,
            litellm_debug_info=self.litellm_debug_info,
            body=self.body,
        )
class UnsupportedParamsError(BadRequestError):
    def __init__(self, **options: Unpack[UnsupportedParamsOptions]) -> None:
        check_options(
            options, _UNSUPPORTED_PARAMS_KEYS, UnsupportedParamsOptions.__required_keys__,
            "UnsupportedParamsError",
        )
        apply_common_fields(self, 400, "UnsupportedParamsError", options)
class InvalidRequestError(base.BadRequestError):
    def __init__(self, message: str, model: str, llm_provider: str) -> None:
        self.status_code = 400
        self.message = message
        self.model = model
        self.llm_provider = llm_provider
        self.response = mock_response(400)
        super().__init__(message=self.message, response=self.response, body=None)
_BAD_REQUEST: TypeAlias = BadRequestError
class LiteLLMUnknownProvider(_BAD_REQUEST):
    def __init__(self, model: str, custom_llm_provider: str | None = None) -> None:
        self.message = _UNKNOWN_PROVIDER_MESSAGE.format(
            model=model, custom_llm_provider=custom_llm_provider
        )
        super().__init__(
            message=self.message,
            model=model,
            llm_provider=custom_llm_provider,
            response=None,
        )
    @override
    def __str__(self) -> str:
        return self.message
