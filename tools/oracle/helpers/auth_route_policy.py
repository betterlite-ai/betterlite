from typing import Final, Literal, NotRequired, TypedDict
from urllib.parse import urlencode

from fastapi import HTTPException
from litellm.proxy._types import (
    LiteLLM_JWTAuth,
    LiteLLM_UserTable,
    LitellmUserRoles,
    ProxyException,
    UserAPIKeyAuth,
)
from litellm.proxy.auth import auth_checks, auth_exception_handler
from litellm.proxy.auth.route_checks import RouteChecks
from pydantic import JsonValue
from starlette.requests import Request
from starlette.types import Scope


class KeyFields(TypedDict):
    user_id: NotRequired[str | None]
    allowed_routes: NotRequired[list[JsonValue] | None]
    metadata: NotRequired[dict[str, JsonValue] | None]
    team_metadata: NotRequired[dict[str, JsonValue] | None]
    permissions: NotRequired[dict[str, JsonValue] | None]


class UserFields(TypedDict):
    user_id: str
    user_role: NotRequired[str | None]


class Input(TypedDict):
    function: Literal["route_allowed", "viewer_access", "masked_user_id", "jwt_allowed_routes"]
    route: NotRequired[str]
    method: NotRequired[str | None]
    user_id_query: NotRequired[str | None]
    body: NotRequired[dict[str, JsonValue]]
    token: NotRequired[KeyFields | None]
    user: NotRequired[UserFields | None]
    admin_only_routes: NotRequired[list[str]]
    team_allowed_routes: NotRequired[list[str] | None]
    admin_allowed_routes: NotRequired[list[str]]
    role: NotRequired[str | None]
    masked: NotRequired[str]


class Denial(TypedDict):
    status: int
    error_type: str
    param: str | None
    code: str


class RouteAnswer(TypedDict):
    allowed: bool
    error: Denial | None


class MaskAnswer(TypedDict):
    masked: str


Answer = RouteAnswer | MaskAnswer


def run(input_value: Input) -> Answer:
    function = input_value["function"]
    if function == "route_allowed":
        return route_allowed(input_value)
    if function == "viewer_access":
        return viewer_access(input_value)
    if function == "masked_user_id":
        return {"masked": masked_user_id(input_value.get("masked", ""))}
    return jwt_allowed_routes(input_value)


def route_allowed(input_value: Input) -> RouteAnswer:
    try:
        allowed = auth_checks._is_api_route_allowed(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L1313-L1344
            route=input_value.get("route", ""),
            request=request_for(input_value.get("method"), input_value.get("user_id_query")),
            request_data=input_value.get("body", {}),
            valid_token=caller_key(input_value.get("token")),
            user_obj=user_row(input_value.get("user")),
        )
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
        return {"allowed": False, "error": refusal(raised)}
    return {"allowed": allowed, "error": None}


def viewer_access(input_value: Input) -> RouteAnswer:
    try:
        RouteChecks._check_proxy_admin_viewer_access(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/route_checks.py#L835-L940
            route=input_value.get("route", ""),
            _user_role=LitellmUserRoles.PROXY_ADMIN_VIEW_ONLY.value,
            request_data=input_value.get("body", {}),
            request=request_for(input_value.get("method"), None),
        )
    except HTTPException as raised:
        return {"allowed": False, "error": refusal(raised)}
    return {"allowed": True, "error": None}


def masked_user_id(user_id: str) -> str:
    return RouteChecks._mask_user_id(user_id=user_id)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/route_checks.py#L213-L232


def jwt_allowed_routes(input_value: Input) -> RouteAnswer:
    role = input_value.get("role")
    if role is None:
        raise ValueError("jwt_allowed_routes needs a role")
    roles = LiteLLM_JWTAuth.model_construct(
        admin_allowed_routes=input_value.get("admin_allowed_routes", []),
        team_allowed_routes=input_value.get("team_allowed_routes"),
    )
    allowed = auth_checks.allowed_routes_check(
        user_role=LitellmUserRoles(role),
        user_route=input_value.get("route", ""),
        litellm_proxy_roles=roles,
    )
    return {"allowed": allowed, "error": None}


def refusal(raised: Exception) -> Denial:
    exception = auth_exception_handler._as_proxy_exception(raised)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
    return denial_of(exception)


def denial_of(exception: ProxyException) -> Denial:
    return {
        "status": int(exception.code),
        "error_type": exception.type,
        "param": exception.param,
        "code": exception.code,
    }


def request_for(method: str | None, user_id_query: str | None) -> Request:
    query: Final = "" if user_id_query is None else urlencode({"user_id": user_id_query})
    scope: Final[Scope] = {
        "type": "http",
        "method": "GET" if method is None else method.upper(),
        "path": "/",
        "query_string": query.encode(),
    }
    return Request(scope)


def caller_key(fields: KeyFields | None) -> UserAPIKeyAuth | None:
    if fields is None:
        return None
    values: dict[str, JsonValue] = {}
    user_id = fields.get("user_id")
    if user_id is not None:
        values["user_id"] = user_id
    routes = fields.get("allowed_routes")
    if routes is not None:
        values["allowed_routes"] = routes
    metadata = fields.get("metadata")
    if metadata is not None:
        values["metadata"] = metadata
    team_metadata = fields.get("team_metadata")
    if team_metadata is not None:
        values["team_metadata"] = team_metadata
    permissions = fields.get("permissions")
    if permissions is not None:
        values["permissions"] = permissions
    return UserAPIKeyAuth.model_construct(_fields_set=None, **values)  # type: ignore[misc, arg-type]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356


def user_row(fields: UserFields | None) -> LiteLLM_UserTable | None:
    if fields is None:
        return None
    return LiteLLM_UserTable(user_id=fields["user_id"], user_role=fields.get("user_role"))
