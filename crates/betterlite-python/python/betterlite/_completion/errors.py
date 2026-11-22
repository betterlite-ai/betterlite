import json
from typing import Final, NoReturn

from pydantic import JsonValue, TypeAdapter
from typing_extensions import NotRequired, TypedDict

from betterlite import _native
from betterlite.exceptions import (
    APIConnectionError, APIError, APIResponseValidationError,
    AuthenticationError, BadGatewayError, BadRequestError, BudgetExceededError,
    ContentPolicyViolationError, ContextWindowExceededError, GuardrailRaisedException,
    InternalServerError, JSONSchemaValidationError, MidStreamFallbackError,
    NotFoundError, PermissionDeniedError, RateLimitError, ServiceUnavailableError,
    Timeout, UnprocessableEntityError, UnsupportedParamsError,
)
from betterlite.exceptions._http import Request, Response


class ErrorPayload(TypedDict):
    type: str
    message: NotRequired[str]
    status_code: NotRequired[int | None]
    llm_provider: NotRequired[str | None]
    model: NotRequired[str | None]
    body: NotRequired[JsonValue]
    headers: NotRequired[dict[str, str] | None]
    num_retries: NotRequired[int | None]
    max_retries: NotRequired[int | None]


ERROR_PAYLOAD: Final[TypeAdapter[ErrorPayload]] = TypeAdapter(ErrorPayload)

# one entry per `type` B5659's NativeError JSON can carry, so an unknown one never becomes generic
EXCEPTION_TYPES: Final[dict[str, type[BaseException]]] = {
    "AuthenticationError": AuthenticationError, "NotFoundError": NotFoundError,
    "PermissionDeniedError": PermissionDeniedError, "BadRequestError": BadRequestError,
    "UnprocessableEntityError": UnprocessableEntityError, "Timeout": Timeout,
    "RateLimitError": RateLimitError, "BudgetExceededError": BudgetExceededError,
    "ContextWindowExceededError": ContextWindowExceededError,
    "ContentPolicyViolationError": ContentPolicyViolationError,
    "ServiceUnavailableError": ServiceUnavailableError, "BadGatewayError": BadGatewayError,
    "InternalServerError": InternalServerError, "APIError": APIError,
    "APIConnectionError": APIConnectionError,
    "APIResponseValidationError": APIResponseValidationError,
    "JSONSchemaValidationError": JSONSchemaValidationError,
    "UnsupportedParamsError": UnsupportedParamsError,
    "GuardrailRaisedException": GuardrailRaisedException,
    "MidStreamFallbackError": MidStreamFallbackError, "Exception": Exception,
}


class UnknownNativeError(Exception):
    payload: str

    def __init__(self, payload: str) -> None:
        self.payload = payload
        super().__init__(payload)


class UnsupportedCallArgument(Exception):
    argument: str

    def __init__(self, argument: str) -> None:
        self.argument = argument
        super().__init__(
            f"{argument} is not supported by betterlite: the completion call runs in Rust"
        )


def _text_value(value: str | None) -> str:
    if value is None:
        return ""
    return value


def _status_value(value: int | None) -> int:
    if value is None:
        return 500
    return value


def _body_content(value: JsonValue) -> bytes | None:
    match value:
        case None:
            return None
        case str():
            return value.encode()
        case bool() | int() | float() | list() | dict():
            return json.dumps(value).encode()


def _response(payload: ErrorPayload) -> Response:
    request = Request(method="POST", url="https://api.openai.com/v1")
    return Response(
        status_code=_status_value(payload.get("status_code")),
        request=request,
        headers=payload.get("headers"),
        content=_body_content(payload.get("body")),
    )


def _build_error(payload: ErrorPayload) -> BaseException:
    message = _text_value(payload.get("message"))
    model = _text_value(payload.get("model"))
    provider = _text_value(payload.get("llm_provider"))
    body = payload.get("body")
    response = _response(payload)
    match payload["type"]:
        case "AuthenticationError":
            return AuthenticationError(message, provider, model, response)
        case "NotFoundError":
            return NotFoundError(message, model, provider, response)
        case "PermissionDeniedError":
            return PermissionDeniedError(message, provider, model, response)
        case "BadRequestError":
            return BadRequestError(message, model, provider, response, body=body)
        case "UnprocessableEntityError":
            return UnprocessableEntityError(message, model, provider, response)
        case "Timeout":
            return Timeout(message, model, provider, headers=payload.get("headers"))
        case "RateLimitError":
            return RateLimitError(
                message, provider, model, headers=payload.get("headers"), body=body
            )
        case "BudgetExceededError":
            return BudgetExceededError(0.0, 0.0, message, provider)
        case "ContextWindowExceededError":
            return ContextWindowExceededError(message, model, provider)
        case "ContentPolicyViolationError":
            return ContentPolicyViolationError(message, model, provider, body=body)
        case "ServiceUnavailableError":
            return ServiceUnavailableError(message, provider, model)
        case "BadGatewayError":
            return BadGatewayError(message, provider, model)
        case "InternalServerError":
            return InternalServerError(message, provider, model, body=body)
        case "APIError":
            return APIError(
                _status_value(payload.get("status_code")), message, provider, model, body=body
            )
        case "APIConnectionError":
            return APIConnectionError(message, provider, model)
        case "APIResponseValidationError":
            return APIResponseValidationError(message, provider, model)
        case "JSONSchemaValidationError":
            return JSONSchemaValidationError(model, provider, message, "")
        case "UnsupportedParamsError":
            return UnsupportedParamsError(message, provider, model)
        case "GuardrailRaisedException":
            return GuardrailRaisedException(
                message=message,
                should_wrap_with_default_message=False,
                status_code=_status_value(payload.get("status_code")),
            )
        case "MidStreamFallbackError":
            return MidStreamFallbackError(message, model, provider)
        case "Exception":
            return Exception(message)
        case _:
            raise UnknownNativeError(payload["type"])


def raise_native_error(error: _native.NativeError) -> NoReturn:
    payload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] not in EXCEPTION_TYPES:
        raise UnknownNativeError(str(error)) from error
    raise _build_error(payload) from error
