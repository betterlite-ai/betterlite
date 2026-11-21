import inspect
from typing import Annotated, Final

import litellm.exceptions as their_errors
import pytest
from pydantic import BaseModel, BeforeValidator, ConfigDict, JsonValue

import betterlite.exceptions as our_errors
from betterlite.exceptions._process import GuardrailOptions
from betterlite.exceptions._server import MidStreamOptions, TimeoutOptions
from betterlite.exceptions._status import (
    BadRequestOptions,
    ImageFetchOptions,
    UnsupportedParamsOptions,
)

_PUBLIC_NAMES: Final[list[str]] = [
    "LITELLM_EXCEPTION_TYPES",
    "VECTOR_STORE_SEARCH_FAILED_CODE",
    "APIConnectionError",
    "APIError",
    "APIResponseValidationError",
    "AuthenticationError",
    "BadGatewayError",
    "BadRequestError",
    "BlockedPiiEntityError",
    "BudgetExceededError",
    "ContentPolicyViolationError",
    "ContextWindowExceededError",
    "GuardrailRaisedException",
    "ImageFetchError",
    "InternalServerError",
    "InvalidRequestError",
    "JSONSchemaValidationError",
    "LiteLLMUnknownProvider",
    "MidStreamFallbackError",
    "MockException",
    "ModelNotMappedError",
    "ModifyResponseException",
    "NotFoundError",
    "OpenAIError",
    "PermissionDeniedError",
    "RateLimitError",
    "RateLimitErrorCategory",
    "RateLimitType",
    "RejectedRequestError",
    "SensitiveDataRouteException",
    "ServiceUnavailableError",
    "Timeout",
    "UnprocessableEntityError",
    "UnsupportedParamsError",
    "VectorStoreSearchError",
    "VectorStoreSearchFailure",
    "validate_rate_limit_category",
    "validate_rate_limit_type",
]
# inspect.Parameter.empty is the default of a parameter that has none.
_EMPTY_DEFAULT: Final[str] = "<class 'inspect._empty'>"
# LiteLLM's VectorStoreSearchFailure at litellm/types/vector_stores.py L104-L108.
_SEARCH_FAILURE_KEYS: Final[set[str]] = {
    "vector_store_id",
    "custom_llm_provider",
    "error",
}
_DEFAULT_TEXT = Annotated[str, BeforeValidator(repr)]
_VALIDATOR_VALUES: Final[list[JsonValue]] = [
    None,
    "",
    "vendor_rate_limit",
    "litellm_rate_limit",
    "tokens",
    "budget",
    "unknown",
    7,
    True,
    ["vendor_rate_limit"],
    {"category": "vendor_rate_limit"},
]


class _ParameterView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    name: str
    kind: _DEFAULT_TEXT
    default: _DEFAULT_TEXT


class _EnumView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    name: str
    value: JsonValue


def _bad_request() -> BaseException:
    return their_errors.BadRequestError(
        message="m", model="model", llm_provider="provider"
    )


def _image_fetch() -> BaseException:
    return their_errors.ImageFetchError(
        message="m", model="model", llm_provider="provider"
    )


def _unsupported_params() -> BaseException:
    return their_errors.UnsupportedParamsError(
        message="m", llm_provider="provider", model="model"
    )


def _timeout() -> BaseException:
    return their_errors.Timeout(
        message="m", model="model", llm_provider="provider"
    )


def _mid_stream() -> BaseException:
    return their_errors.MidStreamFallbackError(
        message="m", model="model", llm_provider="provider"
    )


def _guardrail() -> BaseException:
    return their_errors.GuardrailRaisedException(
        guardrail_name="pii", message="m", blocked_content=True
    )


def _parameter_views(error: BaseException) -> list[_ParameterView]:
    signature = inspect.signature(type(error))
    return [
        _ParameterView.model_validate(parameter) for parameter in signature.parameters.values()
    ]


def _assert_options(
    required: frozenset[str], optional: frozenset[str], theirs: BaseException
) -> None:
    views = _parameter_views(theirs)
    expected_required = {view.name for view in views if view.default == _EMPTY_DEFAULT}
    expected_optional = {view.name for view in views if view.default != _EMPTY_DEFAULT}
    assert required == expected_required
    assert optional == expected_optional


def test_public_names_exact() -> None:
    assert sorted(our_errors.__all__) == sorted(_PUBLIC_NAMES)
    assert len(set(our_errors.__all__)) == len(our_errors.__all__)
    missing = [name for name in _PUBLIC_NAMES if name not in dir(our_errors)]
    assert missing == []


def test_search_code_matches() -> None:
    assert our_errors.VECTOR_STORE_SEARCH_FAILED_CODE == "vector_store_search_failed"
    assert (
        our_errors.VECTOR_STORE_SEARCH_FAILED_CODE
        == their_errors.VECTOR_STORE_SEARCH_FAILED_CODE
    )


def test_search_failure_keys() -> None:
    assert set(our_errors.VectorStoreSearchFailure.__required_keys__) == _SEARCH_FAILURE_KEYS
    assert not our_errors.VectorStoreSearchFailure.__optional_keys__


def test_rate_limit_members() -> None:
    category_ours = [
        (member.name, _EnumView.model_validate(member).value)
        for member in our_errors.RateLimitErrorCategory
    ]
    category_theirs = [
        (member.name, _EnumView.model_validate(member).value)
        for member in their_errors.RateLimitErrorCategory
    ]
    assert category_ours == category_theirs
    type_ours = [
        (member.name, _EnumView.model_validate(member).value)
        for member in our_errors.RateLimitType
    ]
    type_theirs = [
        (member.name, _EnumView.model_validate(member).value)
        for member in their_errors.RateLimitType
    ]
    assert type_ours == type_theirs


@pytest.mark.parametrize("value", _VALIDATOR_VALUES)
def test_category_validation(value: JsonValue) -> None:
    theirs = their_errors.validate_rate_limit_category(value)
    ours = our_errors.validate_rate_limit_category(value)
    assert ours == theirs


@pytest.mark.parametrize("value", _VALIDATOR_VALUES)
def test_type_validation(value: JsonValue) -> None:
    theirs = their_errors.validate_rate_limit_type(value)
    ours = our_errors.validate_rate_limit_type(value)
    assert ours == theirs


def test_category_accepts_enum() -> None:
    theirs = their_errors.validate_rate_limit_category(
        their_errors.RateLimitErrorCategory.VENDOR_BATCH_RATE_LIMIT
    )
    ours = our_errors.validate_rate_limit_category(
        our_errors.RateLimitErrorCategory.VENDOR_BATCH_RATE_LIMIT
    )
    assert ours == theirs == "vendor_batch_rate_limit"


def test_type_accepts_enum() -> None:
    theirs = their_errors.validate_rate_limit_type(their_errors.RateLimitType.MAX_ITERATIONS)
    ours = our_errors.validate_rate_limit_type(our_errors.RateLimitType.MAX_ITERATIONS)
    assert ours == theirs == "max_iterations"


def test_exception_types_order() -> None:
    ours = [cls.__name__ for cls in our_errors.LITELLM_EXCEPTION_TYPES]
    theirs = [cls.__name__ for cls in their_errors.LITELLM_EXCEPTION_TYPES]
    assert ours == theirs


_OPTION_CASES = [
    pytest.param(
        BadRequestOptions.__required_keys__,
        BadRequestOptions.__optional_keys__,
        _bad_request(),
        id="bad-request",
    ),
    pytest.param(
        ImageFetchOptions.__required_keys__,
        ImageFetchOptions.__optional_keys__,
        _image_fetch(),
        id="image-fetch",
    ),
    pytest.param(
        UnsupportedParamsOptions.__required_keys__,
        UnsupportedParamsOptions.__optional_keys__,
        _unsupported_params(),
        id="unsupported-params",
    ),
    pytest.param(
        TimeoutOptions.__required_keys__,
        TimeoutOptions.__optional_keys__,
        _timeout(),
        id="timeout",
    ),
    pytest.param(
        MidStreamOptions.__required_keys__,
        MidStreamOptions.__optional_keys__,
        _mid_stream(),
        id="mid-stream",
    ),
    pytest.param(
        GuardrailOptions.__required_keys__,
        GuardrailOptions.__optional_keys__,
        _guardrail(),
        id="guardrail",
    ),
]


@pytest.mark.parametrize(("required", "optional", "theirs"), _OPTION_CASES)
def test_options_match(
    required: frozenset[str], optional: frozenset[str], theirs: BaseException
) -> None:
    _assert_options(
        required,
        optional,
        theirs,
    )
