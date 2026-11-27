from typing import Final, Literal, TypeAlias

from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.auth.auth_exception_handler import _as_proxy_exception  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_exception_handler.py#L47-L83
from litellm.proxy.auth.auth_method import AuthMethod
from litellm.proxy.auth.network import NetworkContext
from litellm.proxy.auth.resolvers.exceptions import (
    KeyNotFoundError,
    KeyNotInCacheError,
    NoDatabaseConnectionError,
    PrincipalMissingSourceKeyError as MissingSourceKeyError,
)
from litellm.proxy.auth.resolvers.models import CredentialRef
from litellm.proxy.auth.resolvers.store import IdentityStore
from pydantic import BaseModel, Field, JsonValue, TypeAdapter

Row: TypeAlias = dict[str, JsonValue]
JSON_OBJECT: Final[TypeAdapter[Row]] = TypeAdapter(Row)


class Call(BaseModel):
    aspect: Literal["principal", "carrier", "error"]
    key: Row | None = None
    auth_method: AuthMethod = AuthMethod.API_KEY
    issuer: str | None = None
    subject_fallback: str | None = None
    scopes: list[str] = Field(default_factory=list)
    credential_ref: Row | None = None
    network: Row | None = None
    keep_source_key: bool = True
    kind: Literal["missing_key", "no_database", "not_in_cache", "missing_source_key"] | None = None
    hashed_token: str = ""


def _row(value: Row | None) -> Row:
    if value is None:
        raise ValueError("this call needs a key")
    return value


def _principal(fields: Call) -> JsonValue:
    auth = UserAPIKeyAuth.model_validate(_row(fields.key))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356-L3427
    projected = IdentityStore._principal_from_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/resolvers/store.py#L148-L191
        auth,
        auth_method=fields.auth_method,
        issuer=fields.issuer,
        subject_fallback=fields.subject_fallback,
        scopes=fields.scopes,
        credential_ref=None
        if fields.credential_ref is None
        else CredentialRef.model_validate(fields.credential_ref),
        network=None if fields.network is None else NetworkContext.model_validate(fields.network),
    )
    return {"principal": JSON_OBJECT.validate_json(projected.model_dump_json())}


def _key_fields(key: UserAPIKeyAuth) -> JsonValue:
    view = JSON_OBJECT.validate_json(key.model_dump_json())
    return {
        "token": view.get("token"),
        "key_name": view.get("key_name"),
        "user_id": view.get("user_id"),
        "team_id": view.get("team_id"),
        "has_object_permission": view.get("object_permission") is not None,
    }


def _carrier(fields: Call) -> JsonValue:
    auth = UserAPIKeyAuth.model_validate(_row(fields.key))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/_types.py#L3356-L3427
    projected = IdentityStore._principal_from_key(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/resolvers/store.py#L148-L191
        auth,
        auth_method=AuthMethod.API_KEY,
        subject_fallback=auth.token,
        credential_ref=CredentialRef(token_id=auth.token),
    )
    if not fields.keep_source_key:
        projected = projected.model_copy(update={"source_key": None})
    try:
        carried = IdentityStore.key_from_principal(projected)
    except MissingSourceKeyError as error:
        return {"outcome": "missing_source_key", "message": str(error)}
    return {"outcome": "carried", "key": _key_fields(carried)}


def _build_error(fields: Call) -> Exception:
    if fields.kind == "missing_key":
        return KeyNotFoundError(fields.hashed_token)
    if fields.kind == "no_database":
        return NoDatabaseConnectionError()
    if fields.kind == "not_in_cache":
        return KeyNotInCacheError(fields.hashed_token)
    if fields.kind == "missing_source_key":
        return MissingSourceKeyError()
    raise ValueError("an error call needs a kind")


def _error(fields: Call) -> JsonValue:
    proxy = _as_proxy_exception(_build_error(fields))
    return {
        "kind": fields.kind,
        "message": proxy.message,
        "type": proxy.type,
        "param": proxy.param,
        "code": proxy.code,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    call = Call.model_validate(input_value)
    if call.aspect == "principal":
        return _principal(call)
    if call.aspect == "carrier":
        return _carrier(call)
    return _error(call)
