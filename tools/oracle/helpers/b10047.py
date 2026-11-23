from typing import TypedDict

import litellm
from litellm.router import Router
from litellm.types.router import RouterModelGroupAliasItem
from pydantic import JsonValue


class Arguments(TypedDict, total=False):
    model_name: str | None
    model_access_group: str | None
    team_id: str | None


class Input(TypedDict):
    model_list: list[dict[str, JsonValue]]
    model_group_alias: dict[str, str | RouterModelGroupAliasItem]
    arguments: Arguments


def map_value(groups: dict[str, list[str]]) -> dict[str, JsonValue]:
    answer: dict[str, JsonValue] = {}
    for group, names in groups.items():
        listed: list[JsonValue] = []
        listed.extend(names)
        answer[group] = listed
    return answer


def run(input_value: Input) -> JsonValue:
    litellm.suppress_debug_info = True
    router = Router(
        model_list=input_value["model_list"],
        model_group_alias=input_value["model_group_alias"],
    )
    arguments = input_value["arguments"]
    groups = router.get_model_access_groups(
        model_name=arguments.get("model_name"),
        model_access_group=arguments.get("model_access_group"),
        team_id=arguments.get("team_id"),
    )
    return map_value(groups)
