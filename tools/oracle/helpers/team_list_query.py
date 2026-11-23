from typing import NotRequired, TypeAlias, TypedDict

from litellm.models.team import LiteLLM_DeletedTeamTable, LiteLLM_TeamTable, Member
from litellm.proxy.management_endpoints.team_endpoints import _convert_teams_to_response_models  # noqa: E501  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/team_endpoints.py#L5458-L5488
from litellm.types.proxy.management_endpoints.team_endpoints import TeamListItem
from pydantic import JsonValue


class Input(TypedDict):
    teams: list[dict[str, JsonValue]]
    use_deleted_table: bool
    keys_count_by_team: NotRequired[dict[str, int] | None]


# a Prisma row in the versions whose dump is .dict(): model_dump raises, dict returns the stored fields
class DictRow:
    def __init__(self, fields: dict[str, JsonValue]) -> None:
        self._fields = fields

    def model_dump(self) -> dict[str, JsonValue]:
        raise AttributeError(f"{type(self).__name__} carries the fallback dict only")

    def dict(self) -> dict[str, JsonValue]:
        return dict(self._fields)


Row: TypeAlias = LiteLLM_TeamTable | LiteLLM_DeletedTeamTable | DictRow


def build_row(row: dict[str, JsonValue], *, use_deleted_table: bool) -> Row:
    if "members_with_roles" in row and not isinstance(row["members_with_roles"], list):
        return DictRow(row)
    if use_deleted_table:
        return LiteLLM_DeletedTeamTable.model_validate(row)
    return LiteLLM_TeamTable.model_validate(row)


def member_view(member: Member) -> JsonValue:
    return {"user_id": member.user_id, "role": member.role}


def members_view(item: LiteLLM_TeamTable | TeamListItem) -> list[JsonValue]:
    return [member_view(member) for member in item.members_with_roles]


# a list value copies into the JSON shape, since a list[str] is not a JSON value in the type system
def optional_views(values: list[str] | None) -> list[JsonValue] | None:
    if values is None:
        return None
    return [*values]


def item_view(item: TeamListItem | LiteLLM_TeamTable | LiteLLM_DeletedTeamTable) -> JsonValue:
    if isinstance(item, TeamListItem):
        return {
            "kind": "enriched",
            "team_id": item.team_id,
            "team_alias": item.team_alias,
            "members_with_roles": members_view(item),
            "members_count": item.members_count,
            "keys_count": item.keys_count,
            "access_group_models": optional_views(item.access_group_models),
            "access_group_mcp_server_ids": optional_views(item.access_group_mcp_server_ids),
            "access_group_agent_ids": optional_views(item.access_group_agent_ids),
        }
    if isinstance(item, LiteLLM_DeletedTeamTable):
        return {
            "kind": "deleted",
            "team_id": item.team_id,
            "team_alias": item.team_alias,
            "members_with_roles": members_view(item),
            "id": item.id,
            "deleted_by": item.deleted_by,
            "deleted_by_api_key": item.deleted_by_api_key,
            "litellm_changed_by": item.litellm_changed_by,
        }
    return {
        "kind": "live",
        "team_id": item.team_id,
        "team_alias": item.team_alias,
        "members_with_roles": members_view(item),
    }


def run(input_value: Input) -> JsonValue:
    use_deleted_table = input_value["use_deleted_table"]
    rows = [build_row(row, use_deleted_table=use_deleted_table) for row in input_value["teams"]]
    converted = _convert_teams_to_response_models(
        teams=rows,
        use_deleted_table=use_deleted_table,
        keys_count_by_team=input_value.get("keys_count_by_team"),
    )
    return {"teams": [item_view(item) for item in converted]}
