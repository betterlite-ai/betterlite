from collections.abc import Callable
from typing import Any, Literal, NotRequired, TypedDict

import httpx
import litellm
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

# `UNSAFE_PROXY_RESPONSE_HEADERS` from `litellm/proxy/common_request_processing.py`, plus the
# raw-and-prefixed cookie/CORS names the B184 security deviation also strips.
_UNSAFE_RESPONSE_HEADERS = frozenset(
    {
        "content-length",
        "transfer-encoding",
        "content-encoding",
        "content-type",
        "set-cookie",
        "cookie",
        "proxy-authenticate",
        "proxy-authorization",
        "access-control-allow-origin",
        "access-control-allow-credentials",
        "access-control-allow-methods",
        "access-control-allow-headers",
        "access-control-expose-headers",
        "content-security-policy",
        "content-security-policy-report-only",
        "clear-site-data",
        "strict-transport-security",
        "x-frame-options",
        "cross-origin-opener-policy",
        "cross-origin-embedder-policy",
        "cross-origin-resource-policy",
    }
)
_COOKIE_AND_CORS_HEADERS = frozenset(
    {
        "set-cookie",
        "cookie",
        "access-control-allow-origin",
        "access-control-allow-credentials",
        "access-control-allow-methods",
        "access-control-allow-headers",
        "access-control-expose-headers",
    }
)
_LLM_PROVIDER_PREFIX = "llm_provider-"


def _filter_unsafe_headers(headers: dict[str, str]) -> dict[str, str]:
    kept: dict[str, str] = {}
    for name, value in headers.items():
        lower = name.lower()
        if lower in _UNSAFE_RESPONSE_HEADERS:
            continue
        suffix = lower.removeprefix(_LLM_PROVIDER_PREFIX)
        if suffix != lower and suffix in _COOKIE_AND_CORS_HEADERS:
            continue
        kept[name] = value
    return kept


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
    exc = ContentPolicyViolationError(input_value["message"], model, provider)
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


HANDLERS: dict[str, Callable[[Any], object]] = {
    "authentication": _authentication,
    "not_found": _not_found,
    "permission_denied": _permission_denied,
    "bad_request": _bad_request,
    "unprocessable_entity": _unprocessable_entity,
    "context_window_exceeded": _context_window_exceeded,
    "content_policy_violation": _content_policy_violation,
    "service_unavailable": _service_unavailable,
    "bad_gateway": _bad_gateway,
    "internal_server": _internal_server,
    "api_connection": _api_connection,
    "api_response_validation": _api_response_validation,
    "unsupported_params": _unsupported_params,
    "unclassified": _unclassified,
    "timeout": _timeout,
    "rate_limit": _rate_limit,
    "api": _api,
    "mid_stream_fallback": _mid_stream_fallback,
    "budget_exceeded": _budget_exceeded,
    "guardrail_raised": _guardrail_raised,
    "json_schema_validation": _json_schema_validation,
}


def run(input_value: Input) -> object:
    return HANDLERS[input_value["family"]](input_value)
