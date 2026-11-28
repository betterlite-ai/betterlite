import asyncio
from typing import Final, Literal, NotRequired, TypeAlias, TypeVar, TypedDict

from fastapi import HTTPException
from litellm.proxy._types import LiteLLM_JWTAuth, RBAC_ROLES, ScopeMapping
from litellm.proxy.auth import auth_exception_handler
from litellm.proxy.auth.handle_jwt import JWTAuthManager, JWTHandler
from litellm.proxy.auth.model_access_denied import ModelAccessDeniedHTTPException
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue, TypeAdapter

DenialClass = Literal["model_access_denied", "http", "plain"]
RoleName: TypeAlias = Literal["proxy_admin", "team", "internal_user"]

ROLE: Final[TypeAdapter[RBAC_ROLES]] = TypeAdapter(RBAC_ROLES)


class Settings(TypedDict, total=False):
    admin_jwt_scope: str
    admin_allowed_routes: list[str]
    enforce_rbac: bool


class Input(TypedDict):
    function: Literal[
        "can_call_route",
        "can_call_model",
        "check_rbac_role",
        "scope_access",
        "admin_access",
    ]
    role: NotRequired[RoleName | None]
    role_permissions: NotRequired[list[JsonValue] | None]
    enforce_rbac: NotRequired[bool]
    route: NotRequired[str]
    model: NotRequired[str | None]
    scopes: NotRequired[list[str]]
    scope_mappings: NotRequired[list[JsonValue] | None]
    admin_jwt_scope: NotRequired[str | None]
    admin_allowed_routes: NotRequired[list[str] | None]
    token: NotRequired[str | None]
    user_id: NotRequired[str | None]
    user_email: NotRequired[str | None]
    org_id: NotRequired[str | None]
    agent_id: NotRequired[str | None]
    claims: NotRequired[dict[str, JsonValue] | None]


class RawAdminResult(TypedDict):
    is_proxy_admin: bool
    token: str
    team_id: str | None
    user_id: str | None
    user_email: str | None
    end_user_id: str | None
    org_id: str | None
    agent_id: str | None
    jwt_claims: dict[str, JsonValue]


class AdminResult(TypedDict):
    is_proxy_admin: bool
    token_preserved: bool
    team_id: str | None
    user_id: str | None
    user_email: str | None
    end_user_id: str | None
    org_id: str | None
    agent_id: str | None
    jwt_claims: dict[str, JsonValue]


class Answer(TypedDict):
    outcome: Literal["allowed", "denied", "not_admin", "granted"]
    status: int | None
    type: str | None
    param: str | None
    code: str | None
    error_class: DenialClass | None
    message: str | None
    result: AdminResult | None


T = TypeVar("T")


def required(value: T | None, function: str, name: str) -> T:
    if value is None:
        raise ValueError(f"the {function} function needs a {name}")
    return value


def rbac_role(role: RoleName) -> RBAC_ROLES:
    return ROLE.validate_python(role)


def build_handler(settings: Settings) -> JWTHandler:
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(),
        litellm_jwtauth=LiteLLM_JWTAuth(**settings),
        leeway=0,
    )
    return handler


def jwt_settings(input_value: Input) -> Settings:
    enforce = input_value.get("enforce_rbac")
    if enforce is None:
        enforce = False
    settings: Settings = {"enforce_rbac": enforce}
    scope = input_value.get("admin_jwt_scope")
    if scope is not None:
        settings["admin_jwt_scope"] = scope
    routes = input_value.get("admin_allowed_routes")
    if routes is not None:
        settings["admin_allowed_routes"] = routes
    return settings


def general_settings(input_value: Input) -> dict[str, JsonValue]:
    permissions = input_value.get("role_permissions")
    if permissions is None:
        return {}
    return {"role_permissions": permissions}


def request_data(model: str | None) -> dict[str, JsonValue]:
    if model is None:
        return {}
    return {"model": model}


def run(input_value: Input) -> Answer:
    function = input_value["function"]
    if function == "can_call_route":
        return can_call_route(input_value)
    if function == "can_call_model":
        return can_call_model(input_value)
    if function == "check_rbac_role":
        return check_role(input_value)
    if function == "scope_access":
        return scope_access(input_value)
    return admin_access(input_value)


def can_call_route(input_value: Input) -> Answer:
    try:
        JWTAuthManager.can_rbac_role_call_route(
            rbac_role=rbac_role(required(input_value.get("role"), "can_call_route", "role")),
            general_settings=general_settings(input_value),
            route=required(input_value.get("route"), "can_call_route", "route"),
        )
    except ModelAccessDeniedHTTPException as raised:
        return refusal(raised, "model_access_denied")
    except HTTPException as raised:
        return refusal(raised, "http")
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
        return refusal(raised, "plain")
    return plain("allowed")


def can_call_model(input_value: Input) -> Answer:
    try:
        JWTAuthManager.can_rbac_role_call_model(
            rbac_role=rbac_role(required(input_value.get("role"), "can_call_model", "role")),
            general_settings=general_settings(input_value),
            model=input_value.get("model"),
        )
    except ModelAccessDeniedHTTPException as raised:
        return refusal(raised, "model_access_denied")
    except HTTPException as raised:
        return refusal(raised, "http")
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
        return refusal(raised, "plain")
    return plain("allowed")


def check_role(input_value: Input) -> Answer:
    role = input_value.get("role")
    claims = input_value.get("claims")
    token: dict[str, JsonValue] = {} if claims is None else claims
    try:
        asyncio.run(
            JWTAuthManager.check_rbac_role(
                jwt_handler=build_handler(jwt_settings(input_value)),
                jwt_valid_token=token,
                general_settings=general_settings(input_value),
                request_data=request_data(input_value.get("model")),
                route=required(input_value.get("route"), "check_rbac_role", "route"),
                rbac_role=None if role is None else rbac_role(role),
            )
        )
    except ModelAccessDeniedHTTPException as raised:
        return refusal(raised, "model_access_denied")
    except HTTPException as raised:
        return refusal(raised, "http")
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
        return refusal(raised, "plain")
    return plain("allowed")


def scope_access(input_value: Input) -> Answer:
    mappings = input_value.get("scope_mappings")
    validated = TypeAdapter(list[ScopeMapping]).validate_python(
        [] if mappings is None else mappings
    )
    try:
        JWTAuthManager.check_scope_based_access(
            scope_mappings=validated,
            scopes=required(input_value.get("scopes"), "scope_access", "scopes"),
            request_data=request_data(input_value.get("model")),
            general_settings={},
        )
    except ModelAccessDeniedHTTPException as raised:
        return refusal(raised, "model_access_denied")
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
        return refusal(raised, "plain")
    return plain("allowed")


def admin_access(input_value: Input) -> Answer:
    claims = input_value.get("claims")
    try:
        result = asyncio.run(
            JWTAuthManager.check_admin_access(
                jwt_handler=build_handler(jwt_settings(input_value)),
                scopes=required(input_value.get("scopes"), "admin_access", "scopes"),
                route=required(input_value.get("route"), "admin_access", "route"),
                user_id=input_value.get("user_id"),
                org_id=input_value.get("org_id"),
                api_key=required(input_value.get("token"), "admin_access", "token"),
                jwt_valid_token=claims,
                user_email=input_value.get("user_email"),
                agent_id=input_value.get("agent_id"),
            )
        )
    except HTTPException as raised:
        return refusal(raised, "http")
    except Exception as raised:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
        return refusal(raised, "plain")
    if result is None:
        return plain("not_admin")
    return granted(input_value, TypeAdapter(RawAdminResult).validate_python(result))


def granted(input_value: Input, result: RawAdminResult) -> Answer:
    return {
        "outcome": "granted",
        "status": None,
        "type": None,
        "param": None,
        "code": None,
        "error_class": None,
        "message": None,
        "result": {
            "is_proxy_admin": result["is_proxy_admin"],
            "token_preserved": result["token"] == input_value.get("token"),
            "team_id": result["team_id"],
            "user_id": result["user_id"],
            "user_email": result["user_email"],
            "end_user_id": result["end_user_id"],
            "org_id": result["org_id"],
            "agent_id": result["agent_id"],
            "jwt_claims": result["jwt_claims"],
        },
    }


def plain(outcome: Literal["allowed", "not_admin"]) -> Answer:
    return {
        "outcome": outcome,
        "status": None,
        "type": None,
        "param": None,
        "code": None,
        "error_class": None,
        "message": None,
        "result": None,
    }


def refusal(raised: Exception, error_class: DenialClass) -> Answer:
    exception = auth_exception_handler._as_proxy_exception(raised)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L80
    return {
        "outcome": "denied",
        "status": int(exception.code),
        "type": exception.type,
        "param": exception.param,
        "code": exception.code,
        "error_class": error_class,
        "message": exception.message,
        "result": None,
    }
