from typing import Final, TypedDict

from litellm.proxy.auth import auth_checks
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .inputs import JSON, TOKEN_LOOKUP, TOKENS_LOOKUP, Call
from .views import StoredMapping, mapping_rows, stored_mapping


class TokenFilter(TypedDict):
    token: str | dict[str, list[str]]


TOKEN_FILTER: Final[TypeAdapter[TokenFilter]] = TypeAdapter(TokenFilter)


class MappingScript:
    def __init__(self, rows: list[StoredMapping], found: StoredMapping | None) -> None:
        self.rows = rows
        self.found = found
        self.where: list[JsonValue] = []


class MappingTable:
    def __init__(self, script: MappingScript) -> None:
        self.script = script

    async def find_many(
        self, *, where: dict[str, JsonValue], **_options: JsonValue
    ) -> list[StoredMapping]:
        self.script.where.append(JSON.validate_python(TOKEN_FILTER.validate_python(where)))
        return self.script.rows

    async def find_first(
        self, *, where: dict[str, JsonValue], **_options: JsonValue
    ) -> StoredMapping | None:
        self.script.where.append(JSON.validate_python(where))
        return self.script.found


class MappingDb(PrismaWrapper):
    def __init__(self, script: MappingScript) -> None:
        self.script = script

    @override
    def __getattr__(self, name: str) -> MappingTable:
        if name == "litellm_jwtkeymapping":
            return MappingTable(self.script)
        raise AttributeError(name)


class MappingClient(PrismaClient):
    def __init__(self, script: MappingScript) -> None:
        self.db = MappingDb(script)


def claim_pair(call: Call) -> tuple[str, str]:
    claim_name = call.get("claim_name")
    claim_value = call.get("claim_value")
    if claim_name is None or claim_value is None:
        raise ValueError("a mapping call needs claim_name and claim_value")
    return claim_name, claim_value


def cache_key(call: Call) -> JsonValue:
    claim_name, claim_value = claim_pair(call)
    record: dict[str, JsonValue] = {
        "key": auth_checks.jwt_key_mapping_cache_key(claim_name, claim_value, call.get("issuer")),
    }
    return JSON.validate_python(record)


async def mapping(call: Call) -> JsonValue:
    lookup = call.get("lookup")
    if lookup is None:
        raise ValueError("a mapping call needs a lookup")
    found: StoredMapping | None = None
    if lookup["mode"] == "object":
        found_row = lookup.get("found")
        found = None if found_row is None else stored_mapping(found_row)
    script = MappingScript(mapping_rows(call.get("rows")), found)
    client = MappingClient(script)
    record: dict[str, JsonValue] = {}
    match lookup["mode"]:
        case "token":
            token_lookup = TOKEN_LOOKUP.validate_python(lookup)
            keys_one: tuple[str, ...] = await auth_checks.get_jwt_key_mapping_cache_keys_for_token(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3827-L3836
                token_lookup["hashed_token"], client
            )
            record["keys"] = JSON.validate_python(list(keys_one))
        case "tokens":
            tokens_lookup = TOKENS_LOOKUP.validate_python(lookup)
            keys_many = await auth_checks.get_jwt_key_mapping_cache_keys_for_tokens(
                tokens_lookup["hashed_tokens"], client
            )
            record["keys"] = JSON.validate_python(list(keys_many))
        case "object":
            claim_name, claim_value = claim_pair(call)
            raw_token: str | None = await auth_checks.get_jwt_key_mapping_object(  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/auth/auth_checks.py#L3855-L3881
                claim_name, claim_value, client, call.get("issuer")
            )
            record["token"] = raw_token
    record["where"] = script.where[0] if script.where else None
    return JSON.validate_python(record)
