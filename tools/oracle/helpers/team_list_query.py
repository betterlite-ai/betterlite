from typing import Final, NotRequired, Protocol, TypedDict

from litellm.proxy.management_endpoints.team_endpoints import _convert_teams_to_response_models  # pyright: ignore[reportPrivateUsage, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L5458-L5488
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class Input(TypedDict):
    teams: list[dict[str, JsonValue]]
    use_deleted_table: bool
    keys_count_by_team: NotRequired[dict[str, int] | None]


class RowSource(Protocol):
    def model_dump(self) -> dict[str, JsonValue]: ...

    def dict(self) -> dict[str, JsonValue]: ...


# the fallback row LiteLLM reads after model_dump raises
class FallbackRow(RowSource):
    def __init__(self, fields: dict[str, JsonValue]) -> None:
        self._fields = fields

    @override
    def model_dump(self) -> dict[str, JsonValue]:
        raise AttributeError("the fallback row carries the dict only")

    @override
    def dict(self) -> dict[str, JsonValue]:
        return self._fields.copy()


class MemberView(TypedDict):
    user_id: str | None
    role: str


class ItemView(TypedDict):
    team_id: str
    team_alias: str | None
    members_with_roles: list[MemberView]
    members_count: NotRequired[int]
    keys_count: NotRequired[int]
    access_group_models: NotRequired[list[str] | None]
    access_group_mcp_server_ids: NotRequired[list[str] | None]
    access_group_agent_ids: NotRequired[list[str] | None]
    id: NotRequired[str | None]
    deleted_by: NotRequired[str | None]
    deleted_by_api_key: NotRequired[str | None]
    litellm_changed_by: NotRequired[str | None]


ITEM: Final = TypeAdapter(ItemView)


def optional_views(values: list[str] | None) -> list[JsonValue] | None:
    if values is None:
        return None
    return [*values]


def item_view(view: ItemView, *, use_deleted_table: bool) -> JsonValue:
    members: list[JsonValue] = [
        {"user_id": member["user_id"], "role": member["role"]}
        for member in view["members_with_roles"]
    ]
    if use_deleted_table:
        return {
            "kind": "deleted",
            "team_id": view["team_id"],
            "team_alias": view["team_alias"],
            "members_with_roles": members,
            "id": view["id"],
            "deleted_by": view["deleted_by"],
            "deleted_by_api_key": view["deleted_by_api_key"],
            "litellm_changed_by": view["litellm_changed_by"],
        }
    return {
        "kind": "enriched",
        "team_id": view["team_id"],
        "team_alias": view["team_alias"],
        "members_with_roles": members,
        "members_count": view["members_count"],
        "keys_count": view["keys_count"],
        "access_group_models": optional_views(view["access_group_models"]),
        "access_group_mcp_server_ids": optional_views(view["access_group_mcp_server_ids"]),
        "access_group_agent_ids": optional_views(view["access_group_agent_ids"]),
    }


def run(input_value: Input) -> JsonValue:
    use_deleted_table = input_value["use_deleted_table"]
    rows = [FallbackRow(row) for row in input_value["teams"]]
    converted = _convert_teams_to_response_models(
        teams=rows,
        use_deleted_table=use_deleted_table,
        keys_count_by_team=input_value.get("keys_count_by_team"),
    )
    views = [
        item_view(ITEM.validate_json(item.model_dump_json()), use_deleted_table=use_deleted_table)
        for item in converted
    ]
    return {"teams": views}
