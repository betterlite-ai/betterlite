import json
from typing import Final, NoReturn, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

from betterlite import exceptions
from betterlite._native import NativeError
from betterlite.exceptions._http import Request, Response


class ErrorPayload(TypedDict):
    type: str
    message: NotRequired[str]
    status_code: NotRequired[int | None]
    llm_provider: NotRequired[str | None]
    model: NotRequired[str | None]
    body: NotRequired[JsonValue]
    headers: NotRequired[dict[str, str] | None]


ERROR_PAYLOAD: Final[TypeAdapter[ErrorPayload]] = TypeAdapter(ErrorPayload)

# B5659's NativeError JSON carries the exception class name as `type`, so the table maps that name
EXCEPTION_TYPES: Final[dict[str, type[BaseException]]] = {
    error.__name__: error
    for error in (
        exceptions.AuthenticationError, exceptions.NotFoundError,
        exceptions.PermissionDeniedError,
        exceptions.BadRequestError, exceptions.UnprocessableEntityError, exceptions.Timeout,
        exceptions.RateLimitError, exceptions.BudgetExceededError,
        exceptions.ContextWindowExceededError,
        exceptions.ContentPolicyViolationError, exceptions.ServiceUnavailableError,
        exceptions.BadGatewayError, exceptions.InternalServerError, exceptions.APIError,
        exceptions.APIConnectionError, exceptions.APIResponseValidationError,
        exceptions.JSONSchemaValidationError, exceptions.UnsupportedParamsError,
        exceptions.GuardrailRaisedException, exceptions.MidStreamFallbackError,
        Exception,
    )
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


# ast-grep-ignore: no-one-line-functions
def _text_value(value: str | None) -> str:
    return "" if value is None else value


# ast-grep-ignore: no-one-line-functions
def _status_value(value: int | None) -> int:
    return 500 if value is None else value


def _body_content(value: JsonValue) -> bytes | None:
    if value is None:
        return None
    match value:
        case str():
            return value.encode()
        case bool() | int() | float() | list() | dict():
            return json.dumps(value).encode()


def _response(payload: ErrorPayload) -> Response:
    return Response(
        status_code=_status_value(payload.get("status_code")),
        request=Request(method="POST", url="https://api.openai.com/v1"),
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
            return exceptions.AuthenticationError(message, provider, model, response)
        case "NotFoundError":
            return exceptions.NotFoundError(message, model, provider, response)
        case "PermissionDeniedError":
            return exceptions.PermissionDeniedError(message, provider, model, response)
        case "BadRequestError":
            return exceptions.BadRequestError(message, model, provider, response, body=body)
        case "UnprocessableEntityError":
            return exceptions.UnprocessableEntityError(message, model, provider, response)
        case "Timeout":
            return exceptions.Timeout(message, model, provider, headers=payload.get("headers"))
        case "RateLimitError":
            return exceptions.RateLimitError(
                message, provider, model, headers=payload.get("headers"), body=body
            )
        case "BudgetExceededError":
            return exceptions.BudgetExceededError(0.0, 0.0, message, provider)
        case "ContextWindowExceededError":
            return exceptions.ContextWindowExceededError(message, model, provider)
        case "ContentPolicyViolationError":
            return exceptions.ContentPolicyViolationError(message, model, provider, body=body)
        case "ServiceUnavailableError":
            return exceptions.ServiceUnavailableError(message, provider, model)
        case "BadGatewayError":
            return exceptions.BadGatewayError(message, provider, model)
        case "InternalServerError":
            return exceptions.InternalServerError(message, provider, model, body=body)
        case "APIError":
            return exceptions.APIError(
                _status_value(payload.get("status_code")), message, provider, model, body=body
            )
        case "APIConnectionError":
            return exceptions.APIConnectionError(message, provider, model)
        case "APIResponseValidationError":
            return exceptions.APIResponseValidationError(message, provider, model)
        case "JSONSchemaValidationError":
            return exceptions.JSONSchemaValidationError(model, provider, message, "")
        case "UnsupportedParamsError":
            return exceptions.UnsupportedParamsError(message, provider, model)
        case "GuardrailRaisedException":
            return exceptions.GuardrailRaisedException(
                message=message,
                should_wrap_with_default_message=False,
                status_code=_status_value(payload.get("status_code")),
            )
        case "MidStreamFallbackError":
            return exceptions.MidStreamFallbackError(message, model, provider)
        case "Exception":
            return Exception(message)
        case _:
            raise UnknownNativeError(payload["type"])


def raise_native_error(error: NativeError) -> NoReturn:
    payload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] not in EXCEPTION_TYPES:
        raise UnknownNativeError(str(error)) from error
    raise _build_error(payload) from error
