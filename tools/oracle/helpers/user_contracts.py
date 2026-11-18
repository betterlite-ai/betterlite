from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy._types import UpdateUserRequest
from litellm.proxy.management_endpoints.internal_user_endpoints import (
    _normalize_user_info_user_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/internal_user_endpoints.py#L756
    get_user_id_from_request,
)
from litellm.types.proxy.management_endpoints.internal_user_endpoints import (
    BulkDeleteUserRequest,
    BulkDeleteUsersResponse,
    BulkNewUserMeta,
    BulkNewUserRequest,
    BulkNewUserResponse,
    BulkUpdateUserRequest,
    BulkUpdateUserResponse,
    UserCreateResult,
    UserDeleteResult,
    UserListResponse,
    UserSearchWhere,
    UserUpdateResult,
)
from pydantic import BaseModel, JsonValue, TypeAdapter
from starlette.requests import Request

_JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
_NAMES: Final = TypeAdapter(list[JsonValue])

Function: TypeAlias = Literal[
    "bulk_update",
    "bulk_delete",
    "bulk_new",
    "user_update_result",
    "bulk_update_response",
    "user_delete_result",
    "bulk_delete_response",
    "user_create_result",
    "bulk_new_meta",
    "bulk_new_response",
    "user_list_response",
    "user_search_where",
]

UPDATE_KEYS: Final[tuple[str, ...]] = (
    "password",
    "spend",
    "metadata",
    "user_alias",
    "user_role",
    "max_budget",
)

_BULK_UPDATE: Final[TypeAdapter[BulkUpdateUserRequest]] = TypeAdapter(BulkUpdateUserRequest)
_BULK_NEW: Final[TypeAdapter[BulkNewUserRequest]] = TypeAdapter(BulkNewUserRequest)
_SEARCH_WHERE: Final[TypeAdapter[UserSearchWhere]] = TypeAdapter(UserSearchWhere)
_RECORD_MODELS: Final[dict[Function, type[BaseModel]]] = {
    "bulk_delete": BulkDeleteUserRequest,
    "user_update_result": UserUpdateResult,
    "bulk_update_response": BulkUpdateUserResponse,
    "user_delete_result": UserDeleteResult,
    "bulk_delete_response": BulkDeleteUsersResponse,
    "user_create_result": UserCreateResult,
    "bulk_new_meta": BulkNewUserMeta,
    "bulk_new_response": BulkNewUserResponse,
    "user_list_response": UserListResponse,
}


class Call(TypedDict):
    function: Function
    body: JsonValue


class QueryStep(TypedDict):
    query: str
    supplied: str | None


class Input(TypedDict):
    calls: list[Call]
    queries: list[QueryStep]


class QueryOutcome(TypedDict):
    resolved: str | None
    normalized: str | None


class Answers(TypedDict):
    calls: list[JsonValue]
    queries: list[QueryOutcome]


class UpdateUserView(TypedDict):
    user_id: str | None
    user_email: str | None


class TeamRecordView(TypedDict):
    team_id: str
    max_budget_in_team: float | None
    user_role: Literal["user", "admin"]


class NewUserItemView(TypedDict):
    user_id: str | None
    user_email: str | None
    user_alias: str | None
    user_role: str | None
    max_budget: float | None
    teams: list[str] | list[TeamRecordView] | None
    auto_create_key: bool
    send_invite_email: bool | None
    sso_user_id: str | None
    organizations: list[str] | None


_BULK_UPDATE_USER_VIEW: TypeAdapter[UpdateUserView] = TypeAdapter(UpdateUserView)
_BULK_NEW_USER_ITEM_VIEW: TypeAdapter[NewUserItemView] = TypeAdapter(NewUserItemView)


def supplied_names(model_fields_set: set[str], keys: tuple[str, ...]) -> JsonValue:
    names = [name for name in sorted(model_fields_set) if name in keys]
    return _NAMES.validate_python(names)


def update_user_view(request: UpdateUserRequest) -> JsonValue:
    view = _BULK_UPDATE_USER_VIEW.validate_json(request.model_dump_json())
    return {
        "user_id": view["user_id"],
        "user_email": view["user_email"],
        "supplied": supplied_names(request.model_fields_set, UPDATE_KEYS),
    }


def answer(call: Call) -> JsonValue:
    function = call["function"]
    body = call["body"]
    if function == "bulk_update":
        request = _BULK_UPDATE.validate_python(body)
        users: JsonValue = None
        if request.users is not None:
            users = [update_user_view(user) for user in request.users]
        updates: JsonValue = None
        if request.user_updates is not None:
            updates = supplied_names(request.user_updates.model_fields_set, UPDATE_KEYS)
        return {"users": users, "all_users": request.all_users, "user_updates": updates}
    if function == "bulk_new":
        request = _BULK_NEW.validate_python(body)
        return {
            "users": [
                _JSON.validate_json(
                    _BULK_NEW_USER_ITEM_VIEW.dump_json(
                        _BULK_NEW_USER_ITEM_VIEW.validate_json(item.model_dump_json())
                    )
                )
                for item in request.users
            ]
        }
    if function == "user_search_where":
        where_clause = _SEARCH_WHERE.validate_python(body)
        return _JSON.validate_json(_SEARCH_WHERE.dump_json(where_clause))
    return _JSON.validate_json(_RECORD_MODELS[function].model_validate(body).model_dump_json())


def build_request(query: str) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    scope: dict[str, str | bytes | list[tuple[bytes, bytes]]] = {
        "type": "http",
        "scheme": "http",
        "path": "/user/info",
        "query_string": query.encode(),
        "headers": headers,
    }
    return Request(scope=scope)


def query_outcome(step: QueryStep) -> QueryOutcome:
    request = build_request(step["query"])
    return {
        "resolved": get_user_id_from_request(request),
        "normalized": _normalize_user_info_user_id(request, step["supplied"]),
    }


def call_outcome(call: Call) -> JsonValue:
    try:
        return {"ok": True, "value": answer(call)}
    except (ValueError, TypeError, KeyError):
        return {"ok": False}


def run(input_value: Input) -> Answers:
    return {
        "calls": [call_outcome(call) for call in input_value["calls"]],
        "queries": [query_outcome(step) for step in input_value["queries"]],
    }
