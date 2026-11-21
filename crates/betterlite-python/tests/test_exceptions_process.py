import inspect
from typing import Annotated, Final

import litellm
import litellm.exceptions as their_errors
import pytest
from pydantic import BaseModel, BeforeValidator, ConfigDict, JsonValue

import betterlite
import betterlite.exceptions as our_errors

_MESSAGE: Final[str] = "budget has been exceeded"
_MODEL: Final[str] = "gpt-4o"
_PROVIDER: Final[str] = "openai"
_DETECTION: Final[dict[str, JsonValue]] = {"score": 0.5, "labels": ["ssn"]}
_REQUEST_DATA: Final[dict[str, JsonValue]] = {"model": _MODEL, "stream": False}
_ORIGINAL: Final[dict[str, JsonValue]] = {"id": "chatcmpl-1", "usage": {"total_tokens": 12}}
_CHANGED_STATUS: Final[int] = 429
_DEFAULT_TEXT = Annotated[str, BeforeValidator(repr)]
_SIGNATURE_EXEMPT: Final[frozenset[str]] = frozenset(
    {"GuardrailRaisedException", "ModelNotMappedError"}
)
_DROPPED_MODULES: Final[frozenset[str]] = frozenset({"openai", "httpx"})


class _ParameterView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    name: str
    kind: _DEFAULT_TEXT
    default: _DEFAULT_TEXT


class _ProcessView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    message: str | None = None
    status_code: int | None = None
    llm_provider: str | None = None
    model: str | None = None
    num_retries: int | None = None
    max_retries: int | None = None
    litellm_debug_info: str | None = None
    args: tuple[JsonValue, ...] = ()
    current_cost: float | None = None
    max_budget: float | None = None
    entity_type: str | None = None
    entity_id: str | None = None
    category: str | None = None
    rate_limit_type: str | None = None
    guardrail_name: str | None = None
    blocked_content: bool | None = None
    route_to_model: str | None = None
    session_id: str | None = None
    sticky_session_routing: bool | None = None
    detection_info: dict[str, JsonValue] | None = None
    request_data: dict[str, JsonValue] | None = None
    original_response: JsonValue = None


_CASES = [
    pytest.param(
        their_errors.ModelNotMappedError("no model mapped"),
        our_errors.ModelNotMappedError("no model mapped"),
        id="model-not-mapped",
    ),
    pytest.param(
        their_errors.BudgetExceededError(current_cost=1.5, max_budget=2.0),
        our_errors.BudgetExceededError(current_cost=1.5, max_budget=2.0),
        id="budget-defaults",
    ),
    pytest.param(
        their_errors.BudgetExceededError(
            current_cost=3.25,
            max_budget=10.0,
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            entity_type="key",
            entity_id="key_1",
        ),
        our_errors.BudgetExceededError(
            current_cost=3.25,
            max_budget=10.0,
            message=_MESSAGE,
            llm_provider=_PROVIDER,
            entity_type="key",
            entity_id="key_1",
        ),
        id="budget-full",
    ),
    pytest.param(
        their_errors.GuardrailRaisedException(),
        our_errors.GuardrailRaisedException(),
        id="guardrail-defaults",
    ),
    pytest.param(
        their_errors.GuardrailRaisedException(
            guardrail_name="pii", message="card number found", blocked_content=True
        ),
        our_errors.GuardrailRaisedException(
            guardrail_name="pii", message="card number found", blocked_content=True
        ),
        id="guardrail-wrapped",
    ),
    pytest.param(
        their_errors.GuardrailRaisedException(
            guardrail_name="pii",
            message="card number found",
            should_wrap_with_default_message=False,
            status_code=403,
            blocked_content=True,
        ),
        our_errors.GuardrailRaisedException(
            guardrail_name="pii",
            message="card number found",
            should_wrap_with_default_message=False,
            status_code=403,
            blocked_content=True,
        ),
        id="guardrail-raw",
    ),
    pytest.param(
        their_errors.BlockedPiiEntityError(entity_type="EMAIL"),
        our_errors.BlockedPiiEntityError(entity_type="EMAIL"),
        id="blocked-pii-defaults",
    ),
    pytest.param(
        their_errors.BlockedPiiEntityError(
            entity_type="PHONE", guardrail_name="pii", status_code=451
        ),
        our_errors.BlockedPiiEntityError(
            entity_type="PHONE", guardrail_name="pii", status_code=451
        ),
        id="blocked-pii-full",
    ),
    pytest.param(
        their_errors.ModifyResponseException(
            message="rewritten answer", model=_MODEL, request_data=_REQUEST_DATA
        ),
        our_errors.ModifyResponseException(
            message="rewritten answer", model=_MODEL, request_data=_REQUEST_DATA
        ),
        id="modify-response-defaults",
    ),
    pytest.param(
        their_errors.ModifyResponseException(
            message="rewritten answer",
            model=_MODEL,
            request_data=_REQUEST_DATA,
            guardrail_name="bedrock",
            detection_info=_DETECTION,
            original_response=_ORIGINAL,
        ),
        our_errors.ModifyResponseException(
            message="rewritten answer",
            model=_MODEL,
            request_data=_REQUEST_DATA,
            guardrail_name="bedrock",
            detection_info=_DETECTION,
            original_response=_ORIGINAL,
        ),
        id="modify-response-full",
    ),
    pytest.param(
        their_errors.SensitiveDataRouteException(
            route_to_model="local-llama", session_id="session-1"
        ),
        our_errors.SensitiveDataRouteException(
            route_to_model="local-llama", session_id="session-1"
        ),
        id="sensitive-defaults",
    ),
    pytest.param(
        their_errors.SensitiveDataRouteException(
            route_to_model="local-llama",
            session_id="session-2",
            guardrail_name="presidio",
            detection_info=_DETECTION,
            message="sensitive data detected",
            sticky_session_routing=False,
        ),
        our_errors.SensitiveDataRouteException(
            route_to_model="local-llama",
            session_id="session-2",
            guardrail_name="presidio",
            detection_info=_DETECTION,
            message="sensitive data detected",
            sticky_session_routing=False,
        ),
        id="sensitive-full",
    ),
]


def _mro_names(error: BaseException) -> list[str]:
    classes = type(error).__mro__
    return [cls.__name__ for cls in classes if cls.__module__.split(".")[0] not in _DROPPED_MODULES]


def _signature_views(error: BaseException) -> list[_ParameterView]:
    signature = inspect.signature(type(error))
    return [_ParameterView.model_validate(parameter) for parameter in signature.parameters.values()]


def _check(theirs: BaseException, ours: BaseException) -> None:
    assert str(ours) == str(theirs)
    assert repr(ours) == repr(theirs)
    assert _ProcessView.model_validate(ours) == _ProcessView.model_validate(theirs)
    their_names = _mro_names(theirs)
    our_names = _mro_names(ours)
    assert our_names[0] == their_names[0]
    assert [name for name in their_names if name in our_names] == their_names
    if type(theirs).__name__ not in _SIGNATURE_EXEMPT:
        assert _signature_views(ours) == _signature_views(theirs)


@pytest.mark.parametrize(("theirs", "ours"), _CASES)
def test_process_error_matches(theirs: BaseException, ours: BaseException) -> None:
    _check(
        theirs,
        ours,
    )


def test_budget_status_setting() -> None:
    their_status = litellm.budget_exceeded_status_code
    our_status = betterlite.budget_exceeded_status_code
    litellm.budget_exceeded_status_code = _CHANGED_STATUS
    betterlite.budget_exceeded_status_code = _CHANGED_STATUS
    try:
        theirs = their_errors.BudgetExceededError(current_cost=1.0, max_budget=2.0)
        ours = our_errors.BudgetExceededError(current_cost=1.0, max_budget=2.0)
        assert _ProcessView.model_validate(ours).status_code == _CHANGED_STATUS
        assert _ProcessView.model_validate(ours) == _ProcessView.model_validate(theirs)
    finally:
        litellm.budget_exceeded_status_code = their_status
        betterlite.budget_exceeded_status_code = our_status
