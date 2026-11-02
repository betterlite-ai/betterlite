from typing import Literal, TypedDict, cast

from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import ProxyException
from litellm.proxy.auth.auth_checks import (
    _get_user_role,  # pyright: ignore[reportPrivateUsage]
    _is_user_proxy_admin,  # pyright: ignore[reportPrivateUsage]
    effective_user_role,
)
from litellm.proxy.auth.auth_method import AuthMethod
from litellm.proxy.auth.network import NetworkContext
from litellm.proxy.auth.resolvers.exceptions import (
    KeyNotFoundError,
    KeyNotInCacheError,
    NoDatabaseConnectionError,
    PrincipalMissingSourceKeyError,
)
from litellm.proxy.auth.resolvers.models import (
    CredentialRef,
    EndUserIdentity,
    OrganizationIdentity,
    Principal,
    PrincipalType,
    ProjectIdentity,
    TeamIdentity,
    UserIdentity,
)
from litellm.proxy.auth.roles import map_role, team_role
from litellm.types.proxy.auth.auth_checks import UserNotFoundError


class RoleInput(TypedDict):
    aspect: Literal["role"]
    user_role: str | None
    user_present: bool


class RecordsInput(TypedDict):
    aspect: Literal["records"]
    id: str


class PrincipalInput(TypedDict):
    aspect: Literal["principal"]
    subject: str


class NoDatabaseConnectionInput(TypedDict):
    aspect: Literal["error"]
    variant: Literal["no_database_connection"]


class KeyNotInCacheInput(TypedDict):
    aspect: Literal["error"]
    variant: Literal["key_not_in_cache"]
    hashed_token: str


class KeyNotFoundInput(TypedDict):
    aspect: Literal["error"]
    variant: Literal["key_not_found"]
    hashed_token: str


class PrincipalMissingSourceKeyInput(TypedDict):
    aspect: Literal["error"]
    variant: Literal["principal_missing_source_key"]


class UserNotFoundInput(TypedDict):
    aspect: Literal["error"]
    variant: Literal["user_not_found"]
    user_id: str


ErrorInput = (
    NoDatabaseConnectionInput
    | KeyNotInCacheInput
    | KeyNotFoundInput
    | PrincipalMissingSourceKeyInput
    | UserNotFoundInput
)

Input = RoleInput | RecordsInput | PrincipalInput | ErrorInput


def _run_role(fields: RoleInput) -> dict[str, object]:
    value = fields["user_role"]
    role = map_role(value)
    user_obj = LiteLLM_UserTable(user_id="u1", user_role=value) if fields["user_present"] else None
    user_role = _get_user_role(user_obj)
    return {
        "map_role": role.value if role is not None else None,
        "team_role": team_role(value).value,
        "effective_user_role": effective_user_role(value).value,
        "get_user_role": user_role.value if user_role is not None else None,
        "is_user_proxy_admin": _is_user_proxy_admin(user_obj),
    }


def _run_records(fields: RecordsInput) -> dict[str, object]:
    identifier = fields["id"]
    organization = OrganizationIdentity(id=identifier)
    return {
        "user": cast(dict[str, object], UserIdentity(id=identifier).model_dump(mode="json")),
        "organization": cast(dict[str, object], organization.model_dump(mode="json")),
        "project": cast(dict[str, object], ProjectIdentity(id=identifier).model_dump(mode="json")),
        "team": cast(dict[str, object], TeamIdentity(id=identifier).model_dump(mode="json")),
        "end_user": cast(dict[str, object], EndUserIdentity(id=identifier).model_dump(mode="json")),
        "credential_ref": cast(dict[str, object], CredentialRef().model_dump(mode="json")),
        "network": cast(dict[str, object], NetworkContext().model_dump(mode="json")),
    }


def _run_principal(fields: PrincipalInput) -> dict[str, object]:
    principal = Principal(
        principal_type=PrincipalType.HUMAN,
        subject=fields["subject"],
        auth_method=AuthMethod.API_KEY,
    )
    return cast(dict[str, object], principal.model_dump(mode="json"))


def _build_error(fields: ErrorInput) -> Exception:
    match fields["variant"]:
        case "no_database_connection":
            return NoDatabaseConnectionError()
        case "key_not_in_cache":
            return KeyNotInCacheError(fields["hashed_token"])
        case "key_not_found":
            return KeyNotFoundError(fields["hashed_token"])
        case "principal_missing_source_key":
            return PrincipalMissingSourceKeyError()
        case "user_not_found":
            return UserNotFoundError(fields["user_id"])


def _run_error(fields: ErrorInput) -> dict[str, object]:
    error = _build_error(fields)
    status: int | None = None
    error_type: str | None = None
    param: str | None = None
    code: str | None = None
    if isinstance(error, ProxyException):
        error_type = error.type
        param = error.param
        code = error.code
        status = int(error.code) if error.code.isdigit() else None
    return {
        "kind": fields["variant"],
        "status": status,
        "error_type": error_type,
        "param": param,
        "code": code,
    }


def run(input_value: Input) -> dict[str, object]:
    match input_value["aspect"]:
        case "role":
            return _run_role(input_value)
        case "records":
            return _run_records(input_value)
        case "principal":
            return _run_principal(input_value)
        case "error":
            return _run_error(input_value)
