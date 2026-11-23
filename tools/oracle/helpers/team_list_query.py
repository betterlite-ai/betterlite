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


# LiteLLM dumps only the keys each response model declares, so the views differ
class ItemBase(TypedDict):
    team_id: str
    team_alias: str | None
    members_with_roles: list[MemberView]


class LiveItemView(ItemBase):
    members_count: int
    keys_count: int
    access_group_models: list[str] | None
    access_group_mcp_server_ids: list[str] | None
    access_group_agent_ids: list[str] | None


class DeletedItemView(ItemBase):
    id: str | None
    deleted_by: str | None
    deleted_by_api_key: str | None
    litellm_changed_by: str | None


LIVE_ITEM: Final = TypeAdapter(LiveItemView)
DELETED_ITEM: Final = TypeAdapter(DeletedItemView)


def optional_views(values: list[str] | None) -> list[JsonValue] | None:
    if values is None:
        return None
    return [*values]


def members_json(view: ItemBase) -> list[JsonValue]:
    return [
        {"user_id": member["user_id"], "role": member["role"]}
        for member in view["members_with_roles"]
    ]


def base_item(view: ItemBase, kind: str) -> dict[str, JsonValue]:
    return {
        "kind": kind,
        "team_id": view["team_id"],
        "team_alias": view["team_alias"],
        "members_with_roles": members_json(view),
    }


def live_item_view(view: LiveItemView) -> JsonValue:
    item = base_item(view, "enriched")
    item["members_count"] = view["members_count"]
    item["keys_count"] = view["keys_count"]
    item["access_group_models"] = optional_views(view["access_group_models"])
    item["access_group_mcp_server_ids"] = optional_views(view["access_group_mcp_server_ids"])
    item["access_group_agent_ids"] = optional_views(view["access_group_agent_ids"])
    return item


def deleted_item_view(view: DeletedItemView) -> JsonValue:
    item = base_item(view, "deleted")
    item["id"] = view["id"]
    item["deleted_by"] = view["deleted_by"]
    item["deleted_by_api_key"] = view["deleted_by_api_key"]
    item["litellm_changed_by"] = view["litellm_changed_by"]
    return item


def run(input_value: Input) -> JsonValue:
    use_deleted_table = input_value["use_deleted_table"]
    rows = [FallbackRow(row) for row in input_value["teams"]]
    converted = _convert_teams_to_response_models(
        teams=rows,
        use_deleted_table=use_deleted_table,
        keys_count_by_team=input_value.get("keys_count_by_team"),
    )
    if use_deleted_table:
        deleted = [deleted_item_view(DELETED_ITEM.validate_json(item.model_dump_json())) for item in converted]
        return {"teams": deleted}
    live = [live_item_view(LIVE_ITEM.validate_json(item.model_dump_json())) for item in converted]
    return {"teams": live}
