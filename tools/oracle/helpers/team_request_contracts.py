from collections.abc import Callable
from typing import Final, Literal, TypeAlias, TypedDict

import litellm.proxy._types as proxy_types
from litellm.models.team import LiteLLM_DeletedTeamTable, LiteLLM_TeamTable
from litellm.types.proxy.management_endpoints import team_endpoints as teams
from pydantic import BaseModel, JsonValue, TypeAdapter

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
_DUMP: Final = TypeAdapter(dict[str, JsonValue])
_NAMES: Final = TypeAdapter(list[JsonValue])

Operation: TypeAlias = Literal[
    "create", "update", "patch", "member_ref", "budget_patch", "bulk_delete", "bulk_update",
    "permission_get", "permission_update", "bulk_permissions", "permission_get_response",
    "bulk_permissions_response", "team_list", "member_info", "metadata_field", "metadata_schema",
    "bulk_add", "bulk_add_response", "spend_row", "spend_response", "delete_result",
    "budget_update_result",
]
Handler: TypeAlias = Callable[[dict[str, JsonValue]], JsonValue]

_MODELS: Final[dict[str, type[BaseModel]]] = {
    "bulk_permissions_response": teams.BulkUpdateTeamMemberPermissionsResponse,
    "permission_get_response": teams.GetTeamMemberPermissionsResponse,
    "bulk_permissions": teams.BulkUpdateTeamMemberPermissionsRequest,
    "permission_update": teams.UpdateTeamMemberPermissionsRequest,
    "budget_update_result": teams.TeamMemberBudgetUpdateResult, "spend_row": teams.TeamUserSpendRow,
    "permission_get": teams.GetTeamMemberPermissionsRequest,
    "bulk_update": teams.BulkTeamMemberBudgetUpdateRequest,
    "metadata_schema": teams.TeamMetadataSchemaResponse,
    "bulk_delete": teams.BulkTeamMemberDeleteRequest, "spend_response": teams.TeamUserSpendResponse,
    "metadata_field": teams.TeamMetadataFieldSchema, "delete_result": teams.TeamMemberDeleteResult,
}


class Input(TypedDict):
    operation: Operation
    document: dict[str, JsonValue]


def dumped(model: BaseModel) -> JsonValue:
    text = model.model_dump_json()
    return _JSON.validate_json(text)


def dumped_supplied(model: BaseModel) -> JsonValue:
    text = model.model_dump_json(exclude_unset=True)
    return _JSON.validate_json(text)


def dump_member_ref(document: dict[str, JsonValue]) -> JsonValue:
    reference = teams.TeamMemberRef.model_validate(document)
    return {"user_id": reference.user_id, "user_email": reference.user_email}


def dump_budget_patch(document: dict[str, JsonValue]) -> JsonValue:
    patch = teams.TeamMemberBudgetPatch.model_validate(document)
    return {
        "supplied_fields": _NAMES.validate_python(sorted(document)),
        "dump": dumped(patch),
    }


def entry_view(
    item: teams.TeamListItem | LiteLLM_TeamTable | LiteLLM_DeletedTeamTable,
) -> JsonValue:
    dumped_item = _DUMP.validate_json(item.model_dump_json())
    if "members_count" in dumped_item:
        return {
            "kind": "enriched",
            "team_id": dumped_item["team_id"],
            "members_count": dumped_item["members_count"],
            "keys_count": dumped_item["keys_count"],
            "access_group_models": dumped_item["access_group_models"],
            "access_group_mcp_server_ids": dumped_item["access_group_mcp_server_ids"],
            "access_group_agent_ids": dumped_item["access_group_agent_ids"],
        }
    if "deleted_at" in dumped_item:
        return {
            "kind": "deleted",
            "team_id": dumped_item["team_id"],
            "deleted_at": dumped_item["deleted_at"],
            "deleted_by": dumped_item["deleted_by"],
            "id": dumped_item["id"],
        }
    return {"kind": "live", "team_id": dumped_item["team_id"]}


def team_list_answer(document: dict[str, JsonValue]) -> JsonValue:
    response = teams.TeamListResponse.model_validate(document)
    return {
        "teams": [entry_view(item) for item in response.teams],
        "total": response.total,
        "page": response.page,
        "page_size": response.page_size,
        "total_pages": response.total_pages,
    }


def member_info_answer(document: dict[str, JsonValue]) -> JsonValue:
    info = teams.TeamMemberInfoResponse.model_validate(document)
    text = info.model_dump_json(exclude={"spend", "total_spend", "litellm_budget_table"})
    return _DUMP.validate_json(text)


def bulk_add_answer(document: dict[str, JsonValue]) -> JsonValue:
    request = teams.BulkTeamMemberAddRequest.model_validate(document)
    members: JsonValue = None
    if request.members is not None:
        members = [
            {"user_id": member.user_id, "user_email": member.user_email, "role": member.role}
            for member in request.members
        ]
    return {
        "team_id": request.team_id,
        "members": members,
        "all_users": request.all_users,
        "max_budget_in_team": request.max_budget_in_team,
    }


def bulk_add_response_answer(document: dict[str, JsonValue]) -> JsonValue:
    response = teams.BulkTeamMemberAddResponse.model_validate(document)
    return dumped(response)


def supplied_answer(operation: Operation, document: dict[str, JsonValue]) -> JsonValue:
    model: BaseModel
    if operation == "create":
        model = proxy_types.NewTeamRequest.model_validate(document)
    elif operation == "update":
        model = proxy_types.UpdateTeamRequest.model_validate(document)
    else:
        model = proxy_types.PatchTeamRequest.model_validate(document)
    return dumped_supplied(model)


_CUSTOM: Final[dict[str, Handler]] = {
    "member_ref": dump_member_ref, "budget_patch": dump_budget_patch,
    "team_list": team_list_answer, "member_info": member_info_answer,
    "bulk_add": bulk_add_answer, "bulk_add_response": bulk_add_response_answer,
}


def answer(operation: Operation, document: dict[str, JsonValue]) -> JsonValue:
    listed = _MODELS.get(operation)
    if listed is not None:
        return dumped(listed.model_validate(document))
    if operation in ("create", "update", "patch"):
        return supplied_answer(operation, document)
    return _CUSTOM[operation](document)


def run(input_value: Input) -> JsonValue:
    operation = input_value["operation"]
    return answer(operation, input_value["document"])
