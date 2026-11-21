from collections.abc import Sequence
from dataclasses import dataclass
from typing import Final, TypeAlias

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
class VectorStoreSearchFailure(TypedDict):
    vector_store_id: ReadOnly[str]
    custom_llm_provider: ReadOnly[str | None]
    error: ReadOnly[str]
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
    @override
    def __init__(
        self,
        message: str,
        model: str | None,
        llm_provider: str | None,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
        body: JsonValue | bytes | None = None,
    ) -> None:
        self.status_code = 400
        self.message = f"litellm.BadRequestError: {message}"
        self.model = model
        self.llm_provider = llm_provider
        self.litellm_debug_info = litellm_debug_info
        self.max_retries = max_retries
        self.num_retries = num_retries
        if response is not None and response.has_request():
            built = response
        else:
            built = _MINIMAL_ERROR_RESPONSE
        self.response = built
        base.BadRequestError.__init__(self, self.message, response=built, body=body)
class ImageFetchError(BadRequestError):
    @override
    def __init__(
        self,
        message: str,
        model: str | None = None,
        llm_provider: str | None = None,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
        body: JsonValue | bytes | None = None,
    ) -> None:
        super().__init__(
            message=message,
            model=model,
            llm_provider=llm_provider,
            response=response,
            litellm_debug_info=litellm_debug_info,
            max_retries=max_retries,
            num_retries=num_retries,
            body=body,
        )
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
        body: dict[str, JsonValue] = {
            "type": "invalid_request_error",
            "code": VECTOR_STORE_SEARCH_FAILED_CODE,  # ast-grep-ignore: no-long-names
        }
        super().__init__(
            message=(
                "The request could not be grounded in every configured vector store. "
                f"{len(self.failures)} vector store search(es) failed: {detail}"
            ),
            model=model,
            llm_provider=llm_provider,
            body=body,
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
    body: JsonValue | bytes | None = None
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
    @override
    def __init__(
        self,
        message: str,
        llm_provider: str | None = None,
        model: str | None = None,
        status_code: int = 400,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
    ) -> None:
        self.status_code = 400
        self.message = f"litellm.UnsupportedParamsError: {message}"
        self.model = model
        self.llm_provider = llm_provider
        self.litellm_debug_info = litellm_debug_info
        self.max_retries = max_retries
        self.num_retries = num_retries
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
