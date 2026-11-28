from dataclasses import dataclass
from typing import ClassVar, Final

from pydantic import JsonValue
from typing_extensions import override

from betterlite.exceptions import _openai_base as base
from betterlite.exceptions import _process, _status
from betterlite.exceptions._http import Request, Response
from betterlite.exceptions._rate_limit import RateLimitErrorCategory, RateLimitType
from betterlite.exceptions._text import RetryText


def _vertex_response(status_code: int, response: Response | None) -> Response:
    headers = response.headers if response is not None else None
    request = Request(method="POST", url=" https://cloud.google.com/vertex-ai/")
    return Response(status_code=status_code, headers=headers, request=request)
class Timeout(RetryText, base.APITimeoutError):
    status_code: int
    model: str
    llm_provider: str
    litellm_debug_info: str | None
    headers: dict[str, str] | None
    response: Response | None
    @override
    def __init__(
        self,
        message: str,
        model: str,
        llm_provider: str,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
        headers: dict[str, str] | None = None,
        exception_status_code: int | None = None,
        response: Response | None = None,
    ) -> None:
        request = Request(method="POST", url="https://api.openai.com/v1")
        base.APITimeoutError.__init__(self, request)
        self.status_code = 408 if exception_status_code is None else exception_status_code
        self.message = f"litellm.Timeout: {message}"
        self.model = model
        self.llm_provider = llm_provider
        self.litellm_debug_info = litellm_debug_info
        self.max_retries = max_retries
        self.num_retries = num_retries
        self.headers = headers
        if response is not None:
            self.response = response
@dataclass(eq=False, repr=False)
class RateLimitError(RetryText, base.RateLimitError):
    message: str
    llm_provider: str
    model: str
    response: Response | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    category: str | RateLimitErrorCategory = RateLimitErrorCategory.VENDOR_RATE_LIMIT
    rate_limit_type: str | RateLimitType | None = None
    headers: dict[str, str] | None = None
    detail: JsonValue = None
    body: JsonValue | bytes | None = None
    def __post_init__(self) -> None:
        self.status_code = 429
        self.message = f"litellm.RateLimitError: {self.message}"
        category = self.category
        match category:
            case RateLimitErrorCategory():
                self.category = category.value
            case str():
                self.category = category
        rate_limit_type = self.rate_limit_type
        match rate_limit_type:
            case RateLimitType():
                self.rate_limit_type = rate_limit_type.value
            case str() | None:
                self.rate_limit_type = rate_limit_type
        if self.headers is not None:
            self.headers = {name: str(value) for name, value in self.headers.items()}
        self.detail = self.message if self.detail is None else self.detail
        source = self.response
        built = Response(
            status_code=429,
            headers=source.headers if source is not None else None,
            content=source.content if source is not None else None,
            request=Request(method="POST", url=" https://cloud.google.com/vertex-ai/"),
        )
        self.response = built
        super().__init__(self.message, response=built, body=self.body)
        self.code = "429"
        self.type = "throttling_error"
@dataclass(eq=False, repr=False)
class _ServerError(RetryText, base.APIStatusError):
    message: str
    llm_provider: str
    model: str
    response: Response | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    _status_code: ClassVar[int]
    _prefix: ClassVar[str]
    def __post_init__(self) -> None:
        self.status_code = self._status_code
        self.message = f"litellm.{self._prefix}: {self.message}"
        built = _vertex_response(self._status_code, self.response)
        self.response = built
        super().__init__(self.message, response=built, body=None)
class ServiceUnavailableError(_ServerError):
    _status_code: ClassVar[int] = 503
    _prefix: ClassVar[str] = "ServiceUnavailableError"
class BadGatewayError(_ServerError):
    _status_code: ClassVar[int] = 502
    _prefix: ClassVar[str] = "BadGatewayError"
@dataclass(eq=False, repr=False)
class InternalServerError(RetryText, base.InternalServerError):
    message: str
    llm_provider: str
    model: str
    response: Response | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    body: JsonValue | bytes | None = None
    def __post_init__(self) -> None:
        self.status_code = 500
        self.message = f"litellm.InternalServerError: {self.message}"
        built = _vertex_response(500, self.response)
        self.response = built
        super().__init__(self.message, response=built, body=self.body)
        self.type = "internal_server_error"
@dataclass(eq=False, repr=False)
class APIError(RetryText, base.APIError):
    status_code: int
    message: str
    llm_provider: str
    model: str
    request: Request | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    body: JsonValue | bytes | None = None
    def __post_init__(self) -> None:
        self.message = f"litellm.APIError: {self.message}"
        request = self.request
        if request is None:
            request = Request(method="POST", url="https://api.openai.com/v1")
        self.request = request
        super().__init__(self.message, request=request, body=self.body)
@dataclass(eq=False, repr=False)
class APIConnectionError(RetryText, base.APIConnectionError):
    message: str
    llm_provider: str
    model: str
    request: Request | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        self.status_code = 500
        self.message = f"litellm.APIConnectionError: {self.message}"
        request = Request(method="POST", url="https://api.openai.com/v1")
        self.request = request
        super().__init__(message=self.message, request=request)
@dataclass(eq=False, repr=False)
class APIResponseValidationError(RetryText, base.APIResponseValidationError):
    message: str
    llm_provider: str
    model: str
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        message = self.message
        self.message = f"litellm.APIResponseValidationError: {message}"
        request = Request(method="POST", url="https://api.openai.com/v1")
        response = Response(status_code=500, request=request)
        super().__init__(response=response, body=None, message=message)
class JSONSchemaValidationError(APIResponseValidationError):
    def __init__(self, model: str, llm_provider: str, raw_response: str, schema: str) -> None:
        self.raw_response = raw_response
        self.schema = schema
        self.model = model
        message = (
            f"litellm.JSONSchemaValidationError: model={model}, "
            f"returned an invalid response={raw_response}, for schema={schema}.\n"
            "Access raw response with `e.raw_response`"
        )
        self.message = message
        super().__init__(model=model, message=message, llm_provider=llm_provider)
class OpenAIError(base.OpenAIError):
    llm_provider: str
    def __init__(self, original_exception: Exception | None = None) -> None:
        match original_exception:
            case None | Exception():
                super().__init__()
        self.llm_provider = "openai"
@dataclass(eq=False, repr=False)
class MockException(base.APIError):
    status_code: int
    message: str
    llm_provider: str
    model: str
    request: Request | None = None
    litellm_debug_info: str | None = None
    max_retries: int | None = None
    num_retries: int | None = None
    def __post_init__(self) -> None:
        self.message = f"litellm.MockException: {self.message}"
        request = self.request
        if request is None:
            request = Request(method="POST", url="https://api.openai.com/v1")
        self.request = request
        super().__init__(self.message, request=request, body=None)
class MidStreamFallbackError(ServiceUnavailableError):
    original_exception: Exception | None
    generated_content: str
    is_pre_first_chunk: bool
    request: Request | None
    @override
    def __init__(
        self,
        message: str,
        model: str,
        llm_provider: str,
        original_exception: Exception | None = None,
        response: Response | None = None,
        litellm_debug_info: str | None = None,
        max_retries: int | None = None,
        num_retries: int | None = None,
        generated_content: str = "",
        is_pre_first_chunk: bool = False,
    ) -> None:
        original_status = _original_status(original_exception)
        self.status_code = 503 if original_status is None else original_status
        self.message = f"litellm.MidStreamFallbackError: {message}"
        self.model = model
        self.llm_provider = llm_provider
        self.original_exception = original_exception
        self.litellm_debug_info = litellm_debug_info
        self.max_retries = max_retries
        self.num_retries = num_retries
        self.generated_content = generated_content
        self.is_pre_first_chunk = is_pre_first_chunk
        if response is None:
            request = Request(method="POST", url=f"https://{llm_provider}.com/v1/")
            response = Response(status_code=self.status_code, request=request)
        self.response = response
        saved_message = self.message
        super().__init__(
            message=self.message,
            llm_provider=self.llm_provider,
            model=self.model,
            response=response,
            litellm_debug_info=self.litellm_debug_info,
            max_retries=self.max_retries,
            num_retries=self.num_retries,
        )
        self.status_code = 503 if original_status is None else original_status
        self.response = response
        self.request = response.request
        self.message = saved_message
        self.args = (saved_message,)
    @override
    def __str__(self) -> str:
        text = super().__str__()
        if self.original_exception is not None:
            text += (
                f" Original exception: {type(self.original_exception).__name__}: "
                f"{self.original_exception}"
            )
        return text
    @override
    def __repr__(self) -> str:
        return self.__str__()
def _original_status(original_exception: Exception | None) -> int | None:
    match original_exception:
        case Timeout() | APIConnectionError() | APIError() | MockException() as error:
            return error.status_code
        case base.APIStatusError() | base.APIResponseValidationError() as error:
            return error.status_code
        case (
            _process.BudgetExceededError()
            | _process.GuardrailRaisedException()
            | _process.BlockedPiiEntityError()
        ) as error:
            return error.status_code
        case Exception() | None:
            return None
LITELLM_EXCEPTION_TYPES: Final[list[type[BaseException]]] = [
    _status.AuthenticationError, _status.NotFoundError, _status.BadRequestError,
    _status.UnprocessableEntityError, _status.UnsupportedParamsError, Timeout,
    _status.PermissionDeniedError, RateLimitError, _status.ContextWindowExceededError,
    _status.RejectedRequestError, _status.ContentPolicyViolationError, InternalServerError,
    ServiceUnavailableError, BadGatewayError, APIError, APIConnectionError,
    APIResponseValidationError, OpenAIError, InternalServerError, JSONSchemaValidationError,
]
