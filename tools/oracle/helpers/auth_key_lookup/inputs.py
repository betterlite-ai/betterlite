from typing import Final, Literal, NotRequired, TypedDict

from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
Row = dict[str, JsonValue]
Aspect = Literal["cache_key", "mapping", "key_object", "session"]
Mode = Literal["token", "tokens", "object"]
Outcome = Literal["missing", "gone", "outage"]
StepKind = Literal["row", "transport"]


class Reconnect(TypedDict):
    answer: NotRequired[bool]
    timeout: NotRequired[float]
    lock: NotRequired[float]


class Lookup(TypedDict):
    mode: Mode
    hashed_token: NotRequired[str]
    hashed_tokens: NotRequired[list[str]]
    found: NotRequired[Row | None]


class Permission(TypedDict):
    id: str
    fail: NotRequired[bool]


class Step(TypedDict):
    kind: StepKind
    fields: NotRequired[Row]


class MappingFields(TypedDict):
    token: str
    jwt_claim_name: str
    jwt_claim_value: str
    jwt_issuer: NotRequired[str | None]


class TokenLookup(TypedDict):
    mode: Literal["token"]
    hashed_token: str


class TokensLookup(TypedDict):
    mode: Literal["tokens"]
    hashed_tokens: list[str]


class Call(TypedDict):
    aspect: Aspect
    claim_name: NotRequired[str]
    claim_value: NotRequired[str]
    issuer: NotRequired[str | None]
    rows: NotRequired[list[JsonValue]]
    lookup: NotRequired[Lookup]
    hashed_token: NotRequired[str]
    cached: NotRequired[Row | None]
    reconnect: NotRequired[Reconnect]
    permissions: NotRequired[Permission | None]
    check_db_only: NotRequired[bool]
    check_cache_only: NotRequired[bool | None]
    auth: NotRequired[Row]
    user: NotRequired[Row | None]
    user_outcome: NotRequired[Outcome]
    team: NotRequired[Row | None]
    membership: NotRequired[Row | None]


STEP_LIST: Final[TypeAdapter[list[Step]]] = TypeAdapter(list[Step])
MAPPING_FIELDS: Final[TypeAdapter[MappingFields]] = TypeAdapter(MappingFields)
MAPPING_LIST: Final[TypeAdapter[list[MappingFields]]] = TypeAdapter(list[MappingFields])
TOKEN_LOOKUP: Final[TypeAdapter[TokenLookup]] = TypeAdapter(TokenLookup)
TOKENS_LOOKUP: Final[TypeAdapter[TokensLookup]] = TypeAdapter(TokensLookup)
