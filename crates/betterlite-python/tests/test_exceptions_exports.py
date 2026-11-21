from typing import Final

import litellm.exceptions as their_errors
import pytest
from pydantic import BaseModel, ConfigDict, JsonValue

import betterlite.exceptions as our_errors

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
# LiteLLM's VectorStoreSearchFailure at litellm/types/vector_stores.py L104-L108.
_SEARCH_FAILURE_KEYS: Final[set[str]] = {
    "vector_store_id",
    "custom_llm_provider",
    "error",
}
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


class _EnumView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    name: str
    value: JsonValue


class _PositionalView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    message: str | None = None
    status_code: int | None = None
    model: str | None = None
    llm_provider: str | None = None
    guardrail_name: str | None = None
    blocked_content: bool | None = None
    args: tuple[JsonValue, ...] = ()


_POSITIONAL_CASES = [
    pytest.param(
        their_errors.BadRequestError("m", "model", "provider"),
        our_errors.BadRequestError("m", "model", "provider"),
        id="bad-request",
    ),
    pytest.param(
        their_errors.ImageFetchError("m", "model", "provider"),
        our_errors.ImageFetchError("m", "model", "provider"),
        id="image-fetch",
    ),
    pytest.param(
        their_errors.UnsupportedParamsError("m", "provider", "model"),
        our_errors.UnsupportedParamsError("m", "provider", "model"),
        id="unsupported-params",
    ),
    pytest.param(
        their_errors.Timeout("m", "model", "provider"),
        our_errors.Timeout("m", "model", "provider"),
        id="timeout",
    ),
    pytest.param(
        their_errors.MidStreamFallbackError("m", "model", "provider"),
        our_errors.MidStreamFallbackError("m", "model", "provider"),
        id="mid-stream",
    ),
    pytest.param(
        their_errors.GuardrailRaisedException("pii", "m"),
        our_errors.GuardrailRaisedException("pii", "m"),
        id="guardrail",
    ),
]


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


@pytest.mark.parametrize(("theirs", "ours"), _POSITIONAL_CASES)
def test_positional_constructors(theirs: BaseException, ours: BaseException) -> None:
    assert str(ours) == str(theirs)
    assert repr(ours) == repr(theirs)
    assert _PositionalView.model_validate(ours) == _PositionalView.model_validate(theirs)
