import builtins
from dataclasses import dataclass, field
from typing import Final, Unpack

from pydantic import JsonValue
from typing_extensions import TypedDict

import betterlite
from betterlite.exceptions._rate_limit import RateLimitErrorCategory, RateLimitType
from betterlite.exceptions._status import check_options


class GuardrailOptions(TypedDict, total=False):
    guardrail_name: str | None
    message: str
    should_wrap_with_default_message: bool  # ast-grep-ignore: no-long-names
    status_code: int
    blocked_content: bool
_GUARDRAIL_KEYS: Final[frozenset[str]] = (
    GuardrailOptions.__optional_keys__ | GuardrailOptions.__required_keys__
)
class ModelNotMappedError(builtins.Exception):
    pass
@dataclass(eq=False, repr=False)
class BudgetExceededError(builtins.Exception):
    current_cost: float
    max_budget: float
    message: str | None = None
    llm_provider: str | None = None
    entity_type: str | None = None
    entity_id: str | None = None
    status_code: int = field(init=False)
    category: str = field(init=False)
    rate_limit_type: str = field(init=False)
    def __post_init__(self) -> None:
        self.status_code = betterlite.budget_exceeded_status_code
        self.llm_provider = "" if self.llm_provider is None else self.llm_provider
        self.category = RateLimitErrorCategory.LITELLM_RATE_LIMIT.value
        self.rate_limit_type = RateLimitType.BUDGET.value
        message = self.message
        if message is None:
            message = (
                f"Budget has been exceeded! Current cost: {self.current_cost}, "
                f"Max budget: {self.max_budget}"
            )
        self.message = message
        super().__init__(message)
class GuardrailRaisedException(builtins.Exception):
    guardrail_name: str | None
    message: str
    status_code: int
    blocked_content: bool
    def __init__(self, **options: Unpack[GuardrailOptions]) -> None:
        check_options(options, _GUARDRAIL_KEYS, frozenset(), "GuardrailRaisedException")
        guardrail_name = options.get("guardrail_name")
        message = options.get("message")
        if message is None:
            message = ""
        default_message = (
            f"Guardrail raised an exception, Guardrail: {guardrail_name}, Message: {message}"
        )
        self.guardrail_name = guardrail_name
        wrap = options.get("should_wrap_with_default_message")
        if wrap is None:
            wrap = True
        status_code = options.get("status_code")
        self.status_code = 400 if status_code is None else status_code
        blocked = options.get("blocked_content")
        self.blocked_content = False if blocked is None else blocked
        self.message = default_message if wrap else message
        super().__init__(self.message)
class BlockedPiiEntityError(builtins.Exception):
    entity_type: str
    guardrail_name: str | None
    status_code: int
    message: str
    def __init__(
        self,
        entity_type: str,
        guardrail_name: str | None = None,
        status_code: int = 400,
    ) -> None:
        self.entity_type = entity_type
        self.guardrail_name = guardrail_name
        self.status_code = status_code
        self.message = (
            f"Blocked entity detected: {entity_type} by Guardrail: {guardrail_name}. "
            "This entity is not allowed to be used in this request."
        )
        super().__init__(self.message)
@dataclass(eq=False, repr=False)
class ModifyResponseException(builtins.Exception):
    message: str
    model: str
    request_data: dict[str, JsonValue]
    guardrail_name: str | None = None
    detection_info: dict[str, JsonValue] | None = None
    original_response: JsonValue = None
    def __post_init__(self) -> None:
        detection_info = self.detection_info
        self.detection_info = {} if detection_info is None else detection_info
        super().__init__(self.message)
@dataclass(eq=False, repr=False)
class SensitiveDataRouteException(builtins.Exception):
    route_to_model: str
    session_id: str
    guardrail_name: str | None = None
    detection_info: dict[str, JsonValue] | None = None
    message: str | None = None
    sticky_session_routing: bool = True
    def __post_init__(self) -> None:
        detection_info = self.detection_info
        self.detection_info = {} if detection_info is None else detection_info
        message = self.message
        if message is None:
            message = (
                f"Sensitive data detected by {self.guardrail_name}. "
                f"Routing to model: {self.route_to_model}"
            )
        self.message = message
        super().__init__(message)
