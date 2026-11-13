from typing import Final, Literal, TypedDict

from litellm.proxy.guardrails.guardrail_hooks.tool_permission import ToolPermissionGuardrail
from litellm.types.proxy.guardrails.guardrail_hooks.tool_permission import (
    ToolPermissionGuardrailConfigModel,
)
from pydantic import JsonValue, TypeAdapter


class RuleOutput(TypedDict):
    id: str
    tool_name: str | None
    tool_type: str | None
    decision: str
    allowed_param_patterns: dict[str, str] | None


class Input(TypedDict):
    rules: list[dict[str, JsonValue]]
    default_action: Literal["allow", "deny"]
    on_disallowed_action: Literal["block", "rewrite"]


class Output(TypedDict):
    rules: list[RuleOutput]
    default_action: str
    on_disallowed_action: str
    ui_friendly_name: str


RULE_OUTPUT: Final = TypeAdapter(RuleOutput)


def run(input_value: Input) -> Output:
    guardrail = ToolPermissionGuardrail(
        rules=input_value["rules"],
        default_action=input_value["default_action"],
        on_disallowed_action=input_value["on_disallowed_action"],
    )
    return {
        "rules": [RULE_OUTPUT.validate_json(rule.model_dump_json()) for rule in guardrail.rules],
        "default_action": guardrail.default_action,
        "on_disallowed_action": guardrail.on_disallowed_action,
        "ui_friendly_name": ToolPermissionGuardrailConfigModel.ui_friendly_name(),
    }
