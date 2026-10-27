from collections.abc import Mapping
from typing import Literal, TypedDict, cast

from litellm.types.agents import AgentKillSwitchAuth, AgentKillSwitchConfig, AgentResponse
from litellm.types.proxy.agent_identity import AgentIdentityBinding, ManagedAgentContext
from litellm.types.proxy.carried_budget_state import (
    KeyBudgetSnapshot,
    OrgBudgetSnapshot,
    TeamBudgetSnapshot,
    UserBudgetSnapshot,
)
from pydantic import BaseModel, TypeAdapter


class Input(TypedDict):
    function: Literal[
        "kill_auth", "kill_config", "agent_response", "identity_binding",
        "managed_context", "key_snapshot", "team_snapshot", "user_snapshot", "org_snapshot",
    ]
    value: Mapping[str, object]


def run(input_value: Input) -> dict[str, object]:
    value = input_value["value"]
    model: BaseModel
    match input_value["function"]:
        case "kill_auth":
            model = TypeAdapter(AgentKillSwitchAuth).validate_python(value)
        case "kill_config":
            model = AgentKillSwitchConfig.model_validate(value)
        case "agent_response":
            model = AgentResponse.model_validate(value)
        case "identity_binding":
            model = AgentIdentityBinding.model_validate(value)
        case "managed_context":
            model = ManagedAgentContext.model_validate(value)
        case "key_snapshot":
            model = KeyBudgetSnapshot.model_validate(value)
        case "team_snapshot":
            model = TeamBudgetSnapshot.model_validate(value)
        case "user_snapshot":
            model = UserBudgetSnapshot.model_validate(value)
        case "org_snapshot":
            model = OrgBudgetSnapshot.model_validate(value)
    return cast(dict[str, object], model.model_dump(mode="json", by_alias=True))
