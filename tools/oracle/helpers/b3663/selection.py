import asyncio
from typing import Literal, TypeAlias, TypedDict

import litellm
from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy.guardrails.guardrail_helpers import (
    can_modify_guardrails,
    should_proceed_based_on_metadata,  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/guardrails/guardrail_helpers.py#L26
)
from litellm.types.guardrails import GuardrailItem

from b3663.usage import Json


class TeamInput(TypedDict):
    metadata: dict[str, Json] | None


class CanModifyInput(TypedDict):
    function: Literal["can_modify"]
    team: TeamInput | None


class ProceedInput(TypedDict):
    function: Literal["proceed"]
    data: dict[str, Json]
    guardrail_name: str
    callbacks: dict[str, list[str]]


Input: TypeAlias = CanModifyInput | ProceedInput


def proceed(input_value: ProceedInput) -> Json:
    saved = litellm.guardrail_name_config_map
    litellm.guardrail_name_config_map = {
        name: GuardrailItem(callbacks=callbacks, guardrail_name=name)
        for name, callbacks in input_value["callbacks"].items()
    }
    try:
        applies = asyncio.run(
            should_proceed_based_on_metadata(input_value["data"], input_value["guardrail_name"])
        )
    except KeyError as unknown:
        return {"unknown_guardrail": str(unknown).strip("'\"")}
    finally:
        litellm.guardrail_name_config_map = saved
    return {"proceed": applies}


def run(input_value: Input) -> Json:
    match input_value["function"]:
        case "can_modify":
            team = input_value["team"]
            if team is None:
                return {"can_modify": can_modify_guardrails(None)}
            table = LiteLLM_TeamTable(team_id="team", metadata=team["metadata"])
            return {"can_modify": can_modify_guardrails(table)}
        case "proceed":
            return proceed(input_value)
