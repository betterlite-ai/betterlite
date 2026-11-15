from enum import StrEnum
from typing import ClassVar

from pydantic import ConfigDict, Field, JsonValue

from betterlite.types.utils.base import ResponseObject


class PolicyValidationErrorType(StrEnum):
    INVALID_GUARDRAIL = "invalid_guardrail"
    INVALID_TEAM = "invalid_team"
    INVALID_KEY = "invalid_key"
    INVALID_MODEL = "invalid_model"
    INVALID_INHERITANCE = "invalid_inheritance"
    CIRCULAR_INHERITANCE = "circular_inheritance"
    INVALID_SCOPE = "invalid_scope"
    INVALID_SYNTAX = "invalid_syntax"


class PolicyValidationError(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    policy_name: str
    error_type: PolicyValidationErrorType
    message: str
    field: str | None = None
    value: str | None = None


class PolicyValidationResponse(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    valid: bool
    errors: list[PolicyValidationError] = Field(default_factory=list)
    warnings: list[PolicyValidationError] = Field(default_factory=list)


class PolicyValidateRequest(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="forbid")

    policies: dict[str, JsonValue]
