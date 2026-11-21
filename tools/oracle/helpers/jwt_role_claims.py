from typing import Literal, Protocol, TypeAlias, TypedDict

from litellm.proxy._types import LiteLLM_JWTAuth, LitellmUserRoles
from litellm.proxy.auth.handle_jwt import JWTHandler
from litellm.proxy.common_utils.user_api_key_cache import UserApiKeyCache
from pydantic import JsonValue


class Identity(TypedDict, total=False):
    team_id_jwt_field: str | None
    team_id_default: str | None
    user_id_jwt_field: str | None


class RoleMapping(TypedDict):
    role: str
    internal_role: str


class RoleMap(TypedDict):
    jwt_role: str
    litellm_role: str


class Settings(TypedDict, total=False):
    admin_jwt_scope: str
    user_roles_jwt_field: str | None
    user_allowed_roles: list[str] | None
    roles_jwt_field: str | None
    role_mappings: list[RoleMapping] | None
    jwt_litellm_role_map: list[RoleMap] | None


class Scopes(TypedDict):
    kind: Literal["scopes"]


class UserRoles(TypedDict):
    kind: Literal["user_roles"]
    default_value: list[str] | None


class JwtRole(TypedDict):
    kind: Literal["jwt_role"]
    default_value: list[str] | None


class AllowedRole(TypedDict):
    kind: Literal["is_allowed_user_role"]
    user_roles: list[str] | None


class RbacRole(TypedDict):
    kind: Literal["rbac_role"]


class LitellmRole(TypedDict):
    kind: Literal["litellm_role"]


Projection: TypeAlias = Scopes | UserRoles | JwtRole | AllowedRole | RbacRole | LitellmRole


class Input(TypedDict):
    identity: Identity
    settings: Settings
    claims: dict[str, JsonValue]
    projection: Projection


# the claim methods take a bare dict, so this names the types JWTHandler already satisfies
class RoleHandler(Protocol):
    def get_scopes(self, token: dict[str, JsonValue]) -> list[str]: ...

    def get_user_roles(
        self, token: dict[str, JsonValue], default_value: list[str] | None
    ) -> list[str] | None: ...

    def get_jwt_role(
        self, token: dict[str, JsonValue], default_value: list[str] | None
    ) -> list[str] | None: ...

    def is_allowed_user_role(self, user_roles: list[str] | None) -> bool: ...

    def get_rbac_role(self, token: dict[str, JsonValue]) -> LitellmUserRoles | None: ...

    def map_jwt_role_to_litellm_role(
        self, token: dict[str, JsonValue]
    ) -> LitellmUserRoles | None: ...


def build_handler(input_value: Input) -> RoleHandler:
    handler = JWTHandler()
    handler.update_environment(
        prisma_client=None,
        user_api_key_cache=UserApiKeyCache(),
        litellm_jwtauth=LiteLLM_JWTAuth(**input_value["identity"], **input_value["settings"]),
        leeway=0,
    )
    return handler


def run(input_value: Input) -> JsonValue:
    handler = build_handler(input_value)
    token = input_value["claims"]
    projection = input_value["projection"]
    match projection["kind"]:
        case "scopes":
            return handler.get_scopes(token=token)
        case "user_roles":
            return handler.get_user_roles(token=token, default_value=projection["default_value"])
        case "jwt_role":
            return handler.get_jwt_role(token=token, default_value=projection["default_value"])
        case "is_allowed_user_role":
            return handler.is_allowed_user_role(user_roles=projection["user_roles"])
        case "rbac_role":
            return handler.get_rbac_role(token=token)
        case "litellm_role":
            return handler.map_jwt_role_to_litellm_role(token=token)
