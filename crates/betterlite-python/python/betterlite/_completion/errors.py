import json
from typing import Final, NoReturn

from pydantic import JsonValue, TypeAdapter
from typing_extensions import NotRequired, TypedDict

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


class UnsupportedCallArgumentError(Exception):
    argument: str

    def __init__(self, argument: str) -> None:
        self.argument = argument
        super().__init__(
            f"{argument} is not supported by betterlite: the completion call runs in Rust"
        )


UnsupportedCallArgument = UnsupportedCallArgumentError


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


# one dispatch over the native error types: each class is built with its own constructor's fields
def _build_error(payload: ErrorPayload) -> BaseException:
    message = _text_value(payload.get("message"))
    model = _text_value(payload.get("model"))
    provider = _text_value(payload.get("llm_provider"))
    body = payload.get("body")
    headers = payload.get("headers")
    response = _response(payload)
    error: BaseException | None = None
    match payload["type"]:
        case "AuthenticationError":
            error = exceptions.AuthenticationError(message, provider, model, response)
        case "NotFoundError":
            error = exceptions.NotFoundError(message, model, provider, response)
        case "PermissionDeniedError":
            error = exceptions.PermissionDeniedError(message, provider, model, response)
        case "BadRequestError":
            error = exceptions.BadRequestError(message, model, provider, response, body=body)
        case "UnprocessableEntityError":
            error = exceptions.UnprocessableEntityError(message, model, provider, response)
        case "Timeout":
            error = exceptions.Timeout(message, model, provider, headers=headers)
        case "RateLimitError":
            error = exceptions.RateLimitError(
                message, provider, model, headers=headers, body=body
            )
        case "BudgetExceededError":
            error = exceptions.BudgetExceededError(
                current_cost=0.0, max_budget=0.0, message=message, llm_provider=provider
            )
        case "ContextWindowExceededError":
            error = exceptions.ContextWindowExceededError(
                message=message, model=model, llm_provider=provider
            )
        case "ContentPolicyViolationError":
            error = exceptions.ContentPolicyViolationError(
                message=message, model=model, llm_provider=provider, body=body
            )
        case "GuardrailRaisedException":
            error = exceptions.GuardrailRaisedException(
                message=message,
                should_wrap_with_default_message=False,
                status_code=_status_value(payload.get("status_code")),
            )
        case "ServiceUnavailableError":
            error = exceptions.ServiceUnavailableError(
                message=message, llm_provider=provider, model=model
            )
        case "BadGatewayError":
            error = exceptions.BadGatewayError(
                message=message, llm_provider=provider, model=model
            )
        case "InternalServerError":
            error = exceptions.InternalServerError(
                message=message, llm_provider=provider, model=model, body=body
            )
        case "APIError":
            error = exceptions.APIError(
                status_code=_status_value(payload.get("status_code")),
                message=message,
                llm_provider=provider,
                model=model,
                body=body,
            )
        case "JSONSchemaValidationError":
            error = exceptions.JSONSchemaValidationError(model, provider, message, "")
        case "MidStreamFallbackError":
            error = exceptions.MidStreamFallbackError(message, model, provider)
        case "Exception":
            error = Exception(message)
        case "APIConnectionError":
            error = exceptions.APIConnectionError(message, provider, model)
        case "APIResponseValidationError":
            error = exceptions.APIResponseValidationError(message, provider, model)
        case "UnsupportedParamsError":
            error = exceptions.UnsupportedParamsError(message, provider, model)
        case object():
            pass
    if error is None:
        raise UnknownNativeError(payload["type"])
    return error


def raise_native_error(error: NativeError) -> NoReturn:
    payload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] not in EXCEPTION_TYPES:
        raise UnknownNativeError(str(error)) from error
    raise _build_error(payload) from error
