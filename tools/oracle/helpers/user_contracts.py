from collections.abc import Callable
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy._types import UpdateUserRequest
from litellm.proxy.management_endpoints.internal_user_endpoints import (
    _normalize_user_info_user_id,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/internal_user_endpoints.py#L756
    get_user_id_from_request,
)
from litellm.types.proxy.management_endpoints.internal_user_endpoints import (
    BulkDeleteUserRequest,
    BulkDeleteUsersResponse,
    BulkNewUserItem,
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
from pydantic import JsonValue, TypeAdapter
from starlette.requests import Request
from starlette.types import Scope

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
_BULK_DELETE: Final[TypeAdapter[BulkDeleteUserRequest]] = TypeAdapter(BulkDeleteUserRequest)
_BULK_NEW: Final[TypeAdapter[BulkNewUserRequest]] = TypeAdapter(BulkNewUserRequest)
_UPDATE_RESULT: Final[TypeAdapter[UserUpdateResult]] = TypeAdapter(UserUpdateResult)
_UPDATE_RESPONSE: Final[TypeAdapter[BulkUpdateUserResponse]] = TypeAdapter(BulkUpdateUserResponse)
_DELETE_RESULT: Final[TypeAdapter[UserDeleteResult]] = TypeAdapter(UserDeleteResult)
_DELETE_RESPONSE: Final[TypeAdapter[BulkDeleteUsersResponse]] = TypeAdapter(BulkDeleteUsersResponse)
_CREATE_RESULT: Final[TypeAdapter[UserCreateResult]] = TypeAdapter(UserCreateResult)
_NEW_META: Final[TypeAdapter[BulkNewUserMeta]] = TypeAdapter(BulkNewUserMeta)
_NEW_RESPONSE: Final[TypeAdapter[BulkNewUserResponse]] = TypeAdapter(BulkNewUserResponse)
_LIST_RESPONSE: Final[TypeAdapter[UserListResponse]] = TypeAdapter(UserListResponse)
_SEARCH_WHERE: Final[TypeAdapter[UserSearchWhere]] = TypeAdapter(UserSearchWhere)


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
    answer: dict[str, JsonValue] = {
        "user_id": view["user_id"],
        "user_email": view["user_email"],
        "supplied": supplied_names(request.model_fields_set, UPDATE_KEYS),
    }
    return answer


def new_user_item_view(item: BulkNewUserItem) -> JsonValue:
    view = _BULK_NEW_USER_ITEM_VIEW.validate_json(item.model_dump_json())
    return _JSON.validate_json(_BULK_NEW_USER_ITEM_VIEW.dump_json(view))


def bulk_update_answer(call: Call) -> JsonValue:
    request = _BULK_UPDATE.validate_python(call["body"])
    users: JsonValue = None
    if request.users is not None:
        users = [update_user_view(user) for user in request.users]
    updates: JsonValue = None
    if request.user_updates is not None:
        updates = supplied_names(request.user_updates.model_fields_set, UPDATE_KEYS)
    return {"users": users, "all_users": request.all_users, "user_updates": updates}


def bulk_delete_answer(call: Call) -> JsonValue:
    request = _BULK_DELETE.validate_python(call["body"])
    return _JSON.validate_json(request.model_dump_json())


def bulk_new_answer(call: Call) -> JsonValue:
    request = _BULK_NEW.validate_python(call["body"])
    return {"users": [new_user_item_view(item) for item in request.users]}


def update_result_answer(call: Call) -> JsonValue:
    result = _UPDATE_RESULT.validate_python(call["body"])
    return _JSON.validate_json(result.model_dump_json())


def update_response_answer(call: Call) -> JsonValue:
    response = _UPDATE_RESPONSE.validate_python(call["body"])
    return _JSON.validate_json(response.model_dump_json())


def delete_result_answer(call: Call) -> JsonValue:
    result = _DELETE_RESULT.validate_python(call["body"])
    return _JSON.validate_json(result.model_dump_json())


def delete_response_answer(call: Call) -> JsonValue:
    response = _DELETE_RESPONSE.validate_python(call["body"])
    return _JSON.validate_json(response.model_dump_json())


def create_result_answer(call: Call) -> JsonValue:
    result = _CREATE_RESULT.validate_python(call["body"])
    return _JSON.validate_json(result.model_dump_json())


def new_meta_answer(call: Call) -> JsonValue:
    meta = _NEW_META.validate_python(call["body"])
    return _JSON.validate_json(meta.model_dump_json())


def new_response_answer(call: Call) -> JsonValue:
    response = _NEW_RESPONSE.validate_python(call["body"])
    return _JSON.validate_json(response.model_dump_json())


def list_response_answer(call: Call) -> JsonValue:
    response = _LIST_RESPONSE.validate_python(call["body"])
    return _JSON.validate_json(response.model_dump_json())


def search_where_answer(call: Call) -> JsonValue:
    where_clause = _SEARCH_WHERE.validate_python(call["body"])
    return _JSON.validate_json(_SEARCH_WHERE.dump_json(where_clause))


ANSWERS: Final[dict[Function, Callable[[Call], JsonValue]]] = {
    "bulk_update": bulk_update_answer,
    "bulk_delete": bulk_delete_answer,
    "bulk_new": bulk_new_answer,
    "user_update_result": update_result_answer,
    "bulk_update_response": update_response_answer,
    "user_delete_result": delete_result_answer,
    "bulk_delete_response": delete_response_answer,
    "user_create_result": create_result_answer,
    "bulk_new_meta": new_meta_answer,
    "bulk_new_response": new_response_answer,
    "user_list_response": list_response_answer,
    "user_search_where": search_where_answer,
}


def build_request(query: str) -> Request:
    headers: list[tuple[bytes, bytes]] = []
    scope: Scope = {
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
        return {"ok": True, "value": ANSWERS[call["function"]](call)}
    except (ValueError, TypeError, KeyError):
        return {"ok": False}


def run(input_value: Input) -> Answers:
    return {
        "calls": [call_outcome(call) for call in input_value["calls"]],
        "queries": [query_outcome(step) for step in input_value["queries"]],
    }
