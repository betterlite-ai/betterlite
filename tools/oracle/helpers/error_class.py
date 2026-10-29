from typing import Literal, NotRequired, TypedDict

import httpx
import litellm
from litellm.constants import UNSAFE_PROXY_RESPONSE_HEADERS
from litellm.exceptions import APIConnectionError as LitellmAPIConnectionError
from litellm.exceptions import APIError as LitellmAPIError
from litellm.exceptions import APIResponseValidationError as LitellmAPIResponseValidationError
from litellm.exceptions import (
    AuthenticationError,
    BadGatewayError,
    BadRequestError,
    BudgetExceededError,
    ContentPolicyViolationError,
    ContextWindowExceededError,
    GuardrailRaisedException,
    InternalServerError,
    JSONSchemaValidationError,
    MidStreamFallbackError,
    NotFoundError,
    PermissionDeniedError,
    RateLimitError,
    ServiceUnavailableError,
    Timeout,
    UnprocessableEntityError,
    UnsupportedParamsError,
)


class ErrorContextInput(TypedDict):
    message: str
    model: NotRequired[str | None]
    provider: NotRequired[str | None]
    max_retries: NotRequired[int | None]
    num_retries: NotRequired[int | None]
    body_type: NotRequired[str | None]
    body_param: NotRequired[str | None]
    body_code: NotRequired[str | None]


class RequiredErrorContextInput(TypedDict):
    message: str
    model: str
    provider: str
    max_retries: NotRequired[int | None]
    num_retries: NotRequired[int | None]


class AuthenticationInput(ErrorContextInput):
    family: Literal["authentication"]


class NotFoundInput(ErrorContextInput):
    family: Literal["not_found"]


class PermissionDeniedInput(ErrorContextInput):
    family: Literal["permission_denied"]


class BadRequestInput(ErrorContextInput):
    family: Literal["bad_request"]


class UnprocessableEntityInput(ErrorContextInput):
    family: Literal["unprocessable_entity"]


class ContextWindowExceededInput(ErrorContextInput):
    family: Literal["context_window_exceeded"]


class ContentPolicyViolationInput(ErrorContextInput):
    family: Literal["content_policy_violation"]


class ServiceUnavailableInput(ErrorContextInput):
    family: Literal["service_unavailable"]


class BadGatewayInput(ErrorContextInput):
    family: Literal["bad_gateway"]


class InternalServerInput(ErrorContextInput):
    family: Literal["internal_server"]


class ApiConnectionInput(ErrorContextInput):
    family: Literal["api_connection"]


class ApiResponseValidationInput(ErrorContextInput):
    family: Literal["api_response_validation"]


class UnsupportedParamsInput(ErrorContextInput):
    family: Literal["unsupported_params"]


class UnclassifiedInput(ErrorContextInput):
    family: Literal["unclassified"]


class TimeoutInput(TypedDict):
    family: Literal["timeout"]
    context: ErrorContextInput
    original_status: NotRequired[int | None]


class RateLimitInput(TypedDict):
    family: Literal["rate_limit"]
    context: ErrorContextInput
    category: str
    dimension: NotRequired[str | None]
    headers: NotRequired[dict[str, str]]


class ApiInput(TypedDict):
    family: Literal["api"]
    context: ErrorContextInput
    status: int


class MidStreamFallbackInput(TypedDict):
    family: Literal["mid_stream_fallback"]
    context: RequiredErrorContextInput
    original_status: NotRequired[int | None]
    generated_content: NotRequired[str]
    is_pre_first_chunk: NotRequired[bool]


class BudgetExceededInput(TypedDict):
    family: Literal["budget_exceeded"]
    current_cost: float
    max_budget: float
    message: NotRequired[str | None]
    provider: NotRequired[str | None]
    entity_type: NotRequired[str | None]
    entity_id: NotRequired[str | None]
    status_code: int


class GuardrailRaisedInput(TypedDict):
    family: Literal["guardrail_raised"]
    guardrail_name: NotRequired[str | None]
    message: str
    status_code: int
    blocked_content: bool


class JsonSchemaValidationInput(TypedDict):
    family: Literal["json_schema_validation"]
    model: str
    provider: str
    raw_response: str
    schema: str


Input = (
    AuthenticationInput
    | NotFoundInput
    | PermissionDeniedInput
    | BadRequestInput
    | UnprocessableEntityInput
    | ContextWindowExceededInput
    | ContentPolicyViolationInput
    | ServiceUnavailableInput
    | BadGatewayInput
    | InternalServerInput
    | ApiConnectionInput
    | ApiResponseValidationInput
    | UnsupportedParamsInput
    | UnclassifiedInput
    | TimeoutInput
    | RateLimitInput
    | ApiInput
    | MidStreamFallbackInput
    | BudgetExceededInput
    | GuardrailRaisedInput
    | JsonSchemaValidationInput
)


def _body_from_input(context: ErrorContextInput) -> dict[str, str | None] | None:
    keys = ("body_type", "body_param", "body_code")
    if not any(context.get(key) is not None for key in keys):
        return None
    return {
        "type": context.get("body_type"),
        "param": context.get("body_param"),
        "code": context.get("body_code"),
    }


def _filter_unsafe_headers(headers: dict[str, str]) -> dict[str, str]:
    return {
        name: value
        for name, value in headers.items()
        if name.lower() not in UNSAFE_PROXY_RESPONSE_HEADERS
    }


def _mock_response(status_code: int) -> httpx.Response:
    return httpx.Response(status_code=status_code, request=httpx.Request(method="GET", url="https://litellm.ai"))


def _metadata(
    exc: Exception,
    *,
    category: str | None = None,
    rate_limit_type: str | None = None,
    headers: dict[str, str] | None = None,
) -> object:
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
        "rate_limit_category": category,
        "rate_limit_dimension": rate_limit_type,
        "allowed_response_headers": _filter_unsafe_headers(headers) if headers else {},
        "openai_code": getattr(exc, "code", None),
    }


def _authentication(input_value: AuthenticationInput) -> object:
    exc = AuthenticationError(
        input_value["message"],
        llm_provider=input_value.get("provider"),
        model=input_value.get("model"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _not_found(input_value: NotFoundInput) -> object:
    exc = NotFoundError(
        input_value["message"],
        input_value.get("model"),
        input_value.get("provider"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _permission_denied(input_value: PermissionDeniedInput) -> object:
    exc = PermissionDeniedError(
        input_value["message"],
        input_value.get("provider"),
        input_value.get("model"),
        response=_mock_response(403),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _bad_request(input_value: BadRequestInput) -> object:
    exc = BadRequestError(
        input_value["message"],
        input_value.get("model"),
        input_value.get("provider"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
        body=_body_from_input(input_value),
    )
    return _metadata(exc)


def _unprocessable_entity(input_value: UnprocessableEntityInput) -> object:
    exc = UnprocessableEntityError(
        input_value["message"],
        input_value.get("model"),
        input_value.get("provider"),
        response=_mock_response(422),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _context_window_exceeded(input_value: ContextWindowExceededInput) -> object:
    model, provider = input_value.get("model"), input_value.get("provider")
    exc = ContextWindowExceededError(input_value["message"], model, provider)
    return _metadata(exc)


def _content_policy_violation(input_value: ContentPolicyViolationInput) -> object:
    model, provider = input_value.get("model"), input_value.get("provider")
    exc = ContentPolicyViolationError(
        input_value["message"], model, provider, body=_body_from_input(input_value)
    )
    return _metadata(exc)


def _service_unavailable(input_value: ServiceUnavailableInput) -> object:
    exc = ServiceUnavailableError(
        input_value["message"],
        input_value.get("provider"),
        input_value.get("model"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _bad_gateway(input_value: BadGatewayInput) -> object:
    exc = BadGatewayError(
        input_value["message"],
        input_value.get("provider"),
        input_value.get("model"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _internal_server(input_value: InternalServerInput) -> object:
    exc = InternalServerError(
        input_value["message"],
        input_value.get("provider"),
        input_value.get("model"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
        body=_body_from_input(input_value),
    )
    return _metadata(exc)


def _api_connection(input_value: ApiConnectionInput) -> object:
    exc = LitellmAPIConnectionError(
        input_value["message"],
        input_value.get("provider"),
        input_value.get("model"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _api_response_validation(input_value: ApiResponseValidationInput) -> object:
    exc = LitellmAPIResponseValidationError(
        input_value["message"],
        input_value.get("provider"),
        input_value.get("model"),
        max_retries=input_value.get("max_retries"),
        num_retries=input_value.get("num_retries"),
    )
    return _metadata(exc)


def _unsupported_params(input_value: UnsupportedParamsInput) -> object:
    exc = UnsupportedParamsError(
        input_value["message"],
        llm_provider=input_value.get("provider"),
        model=input_value.get("model"),
    )
    return _metadata(exc)


def _unclassified(input_value: UnclassifiedInput) -> object:
    return _metadata(Exception(input_value["message"]))


def _timeout(input_value: TimeoutInput) -> object:
    context = input_value["context"]
    exc = Timeout(
        context["message"],
        context.get("model"),
        context.get("provider"),
        max_retries=context.get("max_retries"),
        num_retries=context.get("num_retries"),
        exception_status_code=input_value.get("original_status"),
    )
    return _metadata(exc)


def _rate_limit(input_value: RateLimitInput) -> object:
    context = input_value["context"]
    headers = input_value.get("headers") or {}
    exc = RateLimitError(
        context["message"],
        context.get("provider"),
        context.get("model"),
        max_retries=context.get("max_retries"),
        num_retries=context.get("num_retries"),
        category=input_value["category"],
        rate_limit_type=input_value.get("dimension"),
        headers=headers or None,
        body=_body_from_input(context),
    )
    return _metadata(
        exc, category=exc.category, rate_limit_type=exc.rate_limit_type, headers=exc.headers
    )


def _api(input_value: ApiInput) -> object:
    context = input_value["context"]
    exc = LitellmAPIError(
        input_value["status"],
        context["message"],
        context.get("provider"),
        context.get("model"),
        max_retries=context.get("max_retries"),
        num_retries=context.get("num_retries"),
        body=_body_from_input(context),
    )
    return _metadata(exc)


class _UpstreamFallbackError(Exception):
    def __init__(self, status_code: int) -> None:
        super().__init__()
        self.status_code = status_code


def _mid_stream_fallback(input_value: MidStreamFallbackInput) -> object:
    context = input_value["context"]
    original_status = input_value.get("original_status")
    original_exception = (
        _UpstreamFallbackError(original_status) if original_status is not None else None
    )
    exc = MidStreamFallbackError(
        context["message"],
        context["model"],
        context["provider"],
        original_exception=original_exception,
        max_retries=context.get("max_retries"),
        num_retries=context.get("num_retries"),
        generated_content=input_value.get("generated_content", ""),
        is_pre_first_chunk=input_value.get("is_pre_first_chunk", False),
    )
    return _metadata(exc)


def _budget_exceeded(input_value: BudgetExceededInput) -> object:
    previous_status_code = litellm.budget_exceeded_status_code
    litellm.budget_exceeded_status_code = input_value["status_code"]
    try:
        exc = BudgetExceededError(
            input_value["current_cost"],
            input_value["max_budget"],
            message=input_value.get("message"),
            llm_provider=input_value.get("provider"),
            entity_type=input_value.get("entity_type"),
            entity_id=input_value.get("entity_id"),
        )
        return _metadata(exc, category=exc.category, rate_limit_type=exc.rate_limit_type)
    finally:
        litellm.budget_exceeded_status_code = previous_status_code


def _guardrail_raised(input_value: GuardrailRaisedInput) -> object:
    exc = GuardrailRaisedException(
        guardrail_name=input_value.get("guardrail_name"),
        message=input_value["message"],
        should_wrap_with_default_message=False,
        status_code=input_value["status_code"],
        blocked_content=input_value["blocked_content"],
    )
    return _metadata(exc)


def _json_schema_validation(input_value: JsonSchemaValidationInput) -> object:
    exc = JSONSchemaValidationError(
        input_value["model"],
        input_value["provider"],
        input_value["raw_response"],
        input_value["schema"],
    )
    return _metadata(exc)


def run(input_value: Input) -> object:  # noqa: C901, PLR0911, PLR0912 # one branch per exception family
    match input_value["family"]:
        case "authentication":
            return _authentication(input_value)
        case "not_found":
            return _not_found(input_value)
        case "permission_denied":
            return _permission_denied(input_value)
        case "bad_request":
            return _bad_request(input_value)
        case "unprocessable_entity":
            return _unprocessable_entity(input_value)
        case "context_window_exceeded":
            return _context_window_exceeded(input_value)
        case "content_policy_violation":
            return _content_policy_violation(input_value)
        case "service_unavailable":
            return _service_unavailable(input_value)
        case "bad_gateway":
            return _bad_gateway(input_value)
        case "internal_server":
            return _internal_server(input_value)
        case "api_connection":
            return _api_connection(input_value)
        case "api_response_validation":
            return _api_response_validation(input_value)
        case "unsupported_params":
            return _unsupported_params(input_value)
        case "unclassified":
            return _unclassified(input_value)
        case "timeout":
            return _timeout(input_value)
        case "rate_limit":
            return _rate_limit(input_value)
        case "api":
            return _api(input_value)
        case "mid_stream_fallback":
            return _mid_stream_fallback(input_value)
        case "budget_exceeded":
            return _budget_exceeded(input_value)
        case "guardrail_raised":
            return _guardrail_raised(input_value)
        case "json_schema_validation":
            return _json_schema_validation(input_value)
