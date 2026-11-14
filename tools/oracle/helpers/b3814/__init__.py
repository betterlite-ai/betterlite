import asyncio
import json
from collections.abc import Coroutine
from typing import Literal, Protocol, TypedDict, cast

from litellm.proxy.db import tool_registry_writer
from litellm.proxy.db.tool_registry_writer import ToolPolicyRegistry
from litellm.proxy.guardrails.guardrail_hooks.tool_policy.tool_policy_guardrail import (
    ToolPolicyGuardrail,
)
from pydantic import JsonValue, TypeAdapter
from starlette.exceptions import HTTPException
from typing_extensions import override


# LiteLLM's apply_guardrail takes the Any-carrying GenericGuardrailAPIInputs and a bare dict
class AppliesGuardrail(Protocol):
    def apply_guardrail(
        self,
        inputs: dict[str, JsonValue],
        request_data: dict[str, JsonValue],
        input_type: Literal["request", "response"],
    ) -> Coroutine[None, None, dict[str, JsonValue]]: ...


class Input(TypedDict):
    initialized: bool
    input_policies: dict[str, str]
    output_policies: dict[str, str]
    blocked_by_permission: dict[str, list[str]]
    inputs: dict[str, JsonValue]
    input_type: Literal["request", "response"]
    request_data: dict[str, JsonValue]


class Blocked(TypedDict):
    status: int
    detail: JsonValue


class Output(TypedDict):
    blocked: Blocked | None
    inputs: JsonValue | None


class FakeRegistry(ToolPolicyRegistry):
    def __init__(
        self,
        *,
        initialized: bool,
        input_policies: dict[str, str],
        output_policies: dict[str, str],
        blocked_by_permission: dict[str, list[str]],
    ) -> None:
        super().__init__()
        self.initialized = initialized
        self.input_policies = input_policies
        self.output_policies = output_policies
        self.blocked_by_permission = blocked_by_permission

    @override
    def is_initialized(self) -> bool:
        return self.initialized

    @override
    def get_output_policy(self, tool_name: str) -> str:
        policy = self.output_policies.get(tool_name)
        if policy is None:
            return "untrusted"
        return policy

    @override
    def get_effective_policies(
        self,
        tool_names: list[str],
        object_permission_id: str | None = None,
        team_object_permission_id: str | None = None,
    ) -> dict[str, str]:
        blocked: set[str] = set()
        for permission_id in (object_permission_id, team_object_permission_id):
            if permission_id is None:
                continue
            stripped = permission_id.strip()
            if not stripped:
                continue
            listed = self.blocked_by_permission.get(stripped)
            if listed is not None:
                blocked.update(listed)
        policies: dict[str, str] = {}
        for name in tool_names:
            if name in blocked:
                policies[name] = "blocked"
                continue
            policy = self.input_policies.get(name)
            policies[name] = "untrusted" if policy is None else policy
        return policies

    def accessor(self) -> ToolPolicyRegistry:
        return self


def run(input_value: Input) -> Output:
    registry = FakeRegistry(
        initialized=input_value["initialized"],
        input_policies=input_value["input_policies"],
        output_policies=input_value["output_policies"],
        blocked_by_permission=input_value["blocked_by_permission"],
    )
    tool_registry_writer.get_tool_policy_registry = registry.accessor
    guardrail = cast(AppliesGuardrail, ToolPolicyGuardrail())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_hooks/tool_policy/tool_policy_guardrail.py#L124-L130
    request_data = input_value["request_data"]
    adapter: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
    try:
        returned = asyncio.run(
            guardrail.apply_guardrail(
                input_value["inputs"],
                request_data,
                input_value["input_type"],
            )
        )
    # LiteLLM raises FastAPI's HTTPException; Starlette's base carries the refusal dict here
    except HTTPException as error:
        blocked: Blocked = {
            "status": error.status_code,
            "detail": adapter.validate_json(json.dumps(error.detail)),
        }
        return {"blocked": blocked, "inputs": None}
    return {"blocked": None, "inputs": returned}
