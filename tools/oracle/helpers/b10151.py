from typing import Literal, TypedDict

from litellm.models.team import LiteLLM_TeamTable
from litellm.proxy.management_endpoints.team_endpoints import (
    _member_budget_source,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L4056-L4059
    _team_default_budget_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L4036-L4043
)
from pydantic import JsonValue


class DefaultIdCall(TypedDict):
    op: Literal["default_id"]
    metadata: list[dict[str, JsonValue] | None]


class SourceCall(TypedDict):
    op: Literal["source"]
    pairs: list[tuple[str | None, str | None]]


Call = DefaultIdCall | SourceCall


def run(input_value: Call) -> list[str | None]:
    match input_value["op"]:
        case "default_id":
            return [
                _team_default_budget_id(LiteLLM_TeamTable(team_id="t", metadata=metadata))
                for metadata in input_value["metadata"]
            ]
        case "source":
            return [
                _member_budget_source(budget_id, default_id)
                for budget_id, default_id in input_value["pairs"]
            ]
