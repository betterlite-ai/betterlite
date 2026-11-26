from typing import Final, NotRequired, TypedDict

from litellm.proxy._types import ProxyErrorTypes, ProxyException, UserAPIKeyAuth
from pydantic import JsonValue, TypeAdapter

from .inputs import JSON, MAPPING_FIELDS, MAPPING_LIST, MappingFields, Row


class AuthView(TypedDict):
    key_name: NotRequired[str | None]
    user_id: NotRequired[str | None]
    team_id: NotRequired[str | None]
    team_alias: NotRequired[str | None]
    user_role: NotRequired[str | None]
    is_session_token: NotRequired[bool | None]
    models: NotRequired[list[str] | None]
    team_models: NotRequired[list[str] | None]
    object_permission_id: NotRequired[str | None]
    object_permission: NotRequired[JsonValue]


AUTH_VIEW: Final[TypeAdapter[AuthView]] = TypeAdapter(AuthView)


def auth_fields(auth: UserAPIKeyAuth) -> JsonValue:
    view: AuthView = AUTH_VIEW.validate_json(auth.model_dump_json())
    models = view.get("models")
    team_models = view.get("team_models")
    permission = view.get("object_permission")
    record: dict[str, JsonValue] = {
        "key_name": view.get("key_name"),
        "user_id": view.get("user_id"),
        "team_id": view.get("team_id"),
        "team_alias": view.get("team_alias"),
        "user_role": view.get("user_role"),
        "is_session_token": view.get("is_session_token"),
        "models": JSON.validate_python([] if models is None else models),
        "team_models": JSON.validate_python([] if team_models is None else team_models),
        "object_permission_id": view.get("object_permission_id"),
        "has_object_permission": permission is not None,
    }
    return JSON.validate_python(record)


def error_fields(error: ProxyException) -> JsonValue:
    record: dict[str, JsonValue] = {
        "type": ProxyErrorTypes(error.type).value,
        "param": error.param,
        "code": error.code,
    }
    return JSON.validate_python(record)


class StoredMapping:
    def __init__(self, fields: MappingFields) -> None:
        self.token = fields["token"]
        self.jwt_claim_name = fields["jwt_claim_name"]
        self.jwt_claim_value = fields["jwt_claim_value"]
        issuer = fields.get("jwt_issuer")
        self.jwt_issuer = "" if issuer is None else issuer


def stored_mapping(row: Row) -> StoredMapping:
    fields = MAPPING_FIELDS.validate_python(row)
    return StoredMapping(fields)


def mapping_rows(rows: list[JsonValue] | None) -> list[StoredMapping]:
    if rows is None:
        return []
    return [StoredMapping(fields) for fields in MAPPING_LIST.validate_python(rows)]
