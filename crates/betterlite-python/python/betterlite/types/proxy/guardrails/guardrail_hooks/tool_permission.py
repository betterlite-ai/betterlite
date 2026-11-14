from typing import Annotated, Final, Literal, Self

from pydantic import BaseModel, BeforeValidator, JsonValue, field_validator, model_validator
from typing_extensions import override

from betterlite.types.proxy.guardrails.guardrail_hooks.base import GuardrailConfigModel
from betterlite.types.utils.base import Dumpable, ResponseObject


def _lowercase_json(value: JsonValue | Dumpable) -> JsonValue | Dumpable:
    match value:
        case str():
            return value.lower()
        case Dumpable() | bool() | int() | float() | list() | dict() | None:
            return value


class ToolPermissionRule(ResponseObject):
    id: str
    tool_name: str | None = None
    tool_type: str | None = None
    decision: Annotated[Literal["allow", "deny"], BeforeValidator(_lowercase_json)]
    allowed_param_patterns: dict[str, str] | None = None

    @field_validator("tool_name", "tool_type", mode="after")
    @classmethod
    def _blank_to_none(cls, value: str | None) -> str | None:
        if value is None:
            return None
        stripped: Final = value.strip()
        if not stripped:
            return None
        return stripped

    @model_validator(mode="after")
    def _ensure_target_present(self) -> Self:
        if self.tool_name is None and self.tool_type is None:
            raise ValueError("Each rule must specify at least a tool_name or tool_type regex")
        return self


class ToolResult(ResponseObject):
    type: str = "tool_result"
    tool_use_id: str
    content: str
    is_error: bool = True


# LiteLLM names this pydantic model after the builtin exception, and the import path keeps that name
class PermissionError(ResponseObject):  # noqa: A001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/proxy/guardrails/guardrail_hooks/tool_permission.py#L67
    tool_name: str
    rule_id: str | None
    message: str


# ast-grep-ignore: no-long-names
class ToolPermissionGuardrailConfigModel(GuardrailConfigModel[BaseModel]):
    rules: list[ToolPermissionRule] | None = None
    default_action: Annotated[
        Literal["allow", "deny"], BeforeValidator(_lowercase_json)
    ] = "deny"
    on_disallowed_action: Annotated[
        Literal["block", "rewrite"], BeforeValidator(_lowercase_json)
    ] = "block"

    @staticmethod
    @override
    def ui_friendly_name() -> str:
        return "LiteLLM Tool Permission Guardrail"
