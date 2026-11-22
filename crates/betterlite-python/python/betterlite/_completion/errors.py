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


def _response_errors(payload: ErrorPayload) -> BaseException | None:
    message = _text_value(payload.get("message"))
    model = _text_value(payload.get("model"))
    provider = _text_value(payload.get("llm_provider"))
    body = payload.get("body")
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
            error = exceptions.Timeout(message, model, provider, headers=payload.get("headers"))
        case "RateLimitError":
            error = exceptions.RateLimitError(
                message, provider, model, headers=payload.get("headers"), body=body
            )
    return error


def _status_errors(payload: ErrorPayload) -> BaseException | None:
    error: BaseException | None = None
    match payload["type"]:
        case "BudgetExceededError":
            error = exceptions.BudgetExceededError(
                current_cost=0.0,
                max_budget=0.0,
                message=_text_value(payload.get("message")),
                llm_provider=_text_value(payload.get("llm_provider")),
            )
        case "ContextWindowExceededError":
            error = exceptions.ContextWindowExceededError(
                message=_text_value(payload.get("message")),
                model=_text_value(payload.get("model")),
                llm_provider=_text_value(payload.get("llm_provider")),
            )
        case "ContentPolicyViolationError":
            error = exceptions.ContentPolicyViolationError(
                message=_text_value(payload.get("message")),
                model=_text_value(payload.get("model")),
                llm_provider=_text_value(payload.get("llm_provider")),
                body=payload.get("body"),
            )
        case "GuardrailRaisedException":
            error = exceptions.GuardrailRaisedException(
                message=_text_value(payload.get("message")),
                should_wrap_with_default_message=False,
                status_code=_status_value(payload.get("status_code")),
            )
        case "ServiceUnavailableError":
            error = exceptions.ServiceUnavailableError(
                message=_text_value(payload.get("message")),
                llm_provider=_text_value(payload.get("llm_provider")),
                model=_text_value(payload.get("model")),
            )
        case "BadGatewayError":
            error = exceptions.BadGatewayError(
                message=_text_value(payload.get("message")),
                llm_provider=_text_value(payload.get("llm_provider")),
                model=_text_value(payload.get("model")),
            )
        case "InternalServerError":
            error = exceptions.InternalServerError(
                message=_text_value(payload.get("message")),
                llm_provider=_text_value(payload.get("llm_provider")),
                model=_text_value(payload.get("model")),
                body=payload.get("body"),
            )
        case "APIError":
            error = exceptions.APIError(
                status_code=_status_value(payload.get("status_code")),
                message=_text_value(payload.get("message")),
                llm_provider=_text_value(payload.get("llm_provider")),
                model=_text_value(payload.get("model")),
                body=payload.get("body"),
            )
    return error


# the three failures below take LiteLLM's (message, llm_provider, model) order positionally
PLAIN_ERRORS: Final[dict[str, type[BaseException]]] = {
    "APIConnectionError": exceptions.APIConnectionError,
    "APIResponseValidationError": exceptions.APIResponseValidationError,
    "UnsupportedParamsError": exceptions.UnsupportedParamsError,
}


def _plain_errors(payload: ErrorPayload) -> BaseException | None:
    name = payload["type"]
    message = _text_value(payload.get("message"))
    if name == "JSONSchemaValidationError":
        return exceptions.JSONSchemaValidationError(
            _text_value(payload.get("model")),
            _text_value(payload.get("llm_provider")),
            message,
            "",
        )
    if name == "MidStreamFallbackError":
        return exceptions.MidStreamFallbackError(
            message, _text_value(payload.get("model")), _text_value(payload.get("llm_provider"))
        )
    if name == "Exception":
        return Exception(message)
    if name in PLAIN_ERRORS:
        return PLAIN_ERRORS[name](
            message, _text_value(payload.get("llm_provider")), _text_value(payload.get("model"))
        )
    return None


def _build_error(payload: ErrorPayload) -> BaseException:
    # the table is split so each dispatch stays under ruff's branch and complexity limits
    for builder in (_response_errors, _status_errors, _plain_errors):
        error = builder(payload)
        if error is not None:
            return error
    raise UnknownNativeError(payload["type"])


def raise_native_error(error: NativeError) -> NoReturn:
    payload = ERROR_PAYLOAD.validate_json(str(error))
    if payload["type"] not in EXCEPTION_TYPES:
        raise UnknownNativeError(str(error)) from error
    raise _build_error(payload) from error
