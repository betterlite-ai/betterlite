from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

from litellm.models.organization import LiteLLM_OrganizationTable
from litellm.models.team import LiteLLM_TeamTableCachedObj
from litellm.models.team_membership import LiteLLM_TeamMembership
from litellm.models.user import LiteLLM_UserTable
from litellm.proxy._types import LiteLLM_ProjectTableCachedObj, UserAPIKeyAuth
from litellm.proxy.auth import auth_object_prefetch
from litellm.proxy.auth.auth_object_prefetch import (
    AuthObjectRefs,
    _identity_memory_ttl,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_object_prefetch.py#L329
    _iter_entries,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_object_prefetch.py#L175
    _validate_row,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_object_prefetch.py#L245-L258
)
from litellm.proxy.auth.user_api_key_auth import (
    _identity_cache_keys,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/user_api_key_auth.py#L3364-L3377
)
from pydantic import BaseModel, JsonValue, TypeAdapter

RowKind: TypeAlias = Literal[
    "user_row", "team_row", "membership_row", "organization_row", "project_row"
]
TABLES: Final[dict[RowKind, type[BaseModel]]] = {
    "user_row": LiteLLM_UserTable,
    "team_row": LiteLLM_TeamTableCachedObj,
    "membership_row": LiteLLM_TeamMembership,
    "organization_row": LiteLLM_OrganizationTable,
    "project_row": LiteLLM_ProjectTableCachedObj,
}


class Stamp(TypedDict):
    last_refreshed_at: NotRequired[float | None]


STAMP: Final[TypeAdapter[Stamp]] = TypeAdapter(Stamp)


class TokenFields(TypedDict):
    user_id: str | None
    team_id: str | None
    org_id: str | None
    project_id: str | None


class ReferencesInput(TypedDict):
    op: Literal["references_and_entries"]
    management_ttl: float
    organization_ttl: int
    tokens: list[TokenFields]


class CacheKeyCall(TypedDict):
    api_key: str
    end_user_id: str | None
    key_is_resolved: bool


class CacheKeysInput(TypedDict):
    op: Literal["identity_cache_keys"]
    calls: list[CacheKeyCall]


class RowCall(TypedDict):
    row: RowKind
    value: JsonValue


class ValidationInput(TypedDict):
    op: Literal["row_validation"]
    refreshed_at: float
    rows: list[RowCall]


class TtlCall(TypedDict):
    value: JsonValue
    management_ttl: float


class TtlInput(TypedDict):
    op: Literal["identity_ttl"]
    ttls: list[TtlCall]


Input: TypeAlias = ReferencesInput | CacheKeysInput | ValidationInput | TtlInput


def references_of(fields: TokenFields, management_ttl: float) -> JsonValue:
    token = UserAPIKeyAuth(
        user_id=fields["user_id"],
        team_id=fields["team_id"],
        org_id=fields["org_id"],
        project_id=fields["project_id"],
    )
    refs = AuthObjectRefs.from_token(token)
    entries: list[JsonValue] = [
        {
            "key": entry.cache_key,
            "row": entry.row,
            "model": entry.model_type.__name__,
            "ttl": None if entry.ttl is None else float(entry.ttl),
        }
        for entry in _iter_entries(refs, management_ttl)
    ]
    refs_value: dict[str, JsonValue] = {
        "user_id": refs.user_id,
        "team_id": refs.team_id,
        "membership_user_id": refs.membership_user_id,
        "organization_id": refs.organization_id,
        "project_id": refs.project_id,
    }
    return {"refs": refs_value, "entries": entries}


def entries_answer(input_value: ReferencesInput) -> JsonValue:
    saved = auth_object_prefetch.DEFAULT_IN_MEMORY_TTL
    auth_object_prefetch.DEFAULT_IN_MEMORY_TTL = input_value["organization_ttl"]
    try:
        answers: list[JsonValue] = [
            references_of(token, input_value["management_ttl"])
            for token in input_value["tokens"]
        ]
    finally:
        auth_object_prefetch.DEFAULT_IN_MEMORY_TTL = saved
    return answers


def keys_of(call: CacheKeyCall) -> JsonValue:
    keys: list[JsonValue] = []
    keys.extend(
        _identity_cache_keys(
            call["api_key"],
            end_user_id=call["end_user_id"],
            key_is_resolved=call["key_is_resolved"],
        )
    )
    return keys


def cache_keys_answer(input_value: CacheKeysInput) -> JsonValue:
    return [keys_of(call) for call in input_value["calls"]]


def row_answer(call: RowCall, refreshed_at: float) -> JsonValue:
    validated = _validate_row(
        call["value"], TABLES[call["row"]], call["row"], refreshed_at
    )
    if validated is None:
        return {"validated": False, "stamp": None}
    stamp = STAMP.validate_json(validated.model_dump_json())
    return {"validated": True, "stamp": stamp.get("last_refreshed_at")}


def row_answers(input_value: ValidationInput) -> JsonValue:
    return [
        row_answer(call, input_value["refreshed_at"]) for call in input_value["rows"]
    ]


def ttl_answers(input_value: TtlInput) -> JsonValue:
    return [
        float(_identity_memory_ttl(call["value"], call["management_ttl"]))
        for call in input_value["ttls"]
    ]


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "references_and_entries":
        return entries_answer(input_value)
    if input_value["op"] == "identity_cache_keys":
        return cache_keys_answer(input_value)
    if input_value["op"] == "row_validation":
        return row_answers(input_value)
    return ttl_answers(input_value)
