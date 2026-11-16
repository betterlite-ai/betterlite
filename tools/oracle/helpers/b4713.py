import asyncio
from datetime import UTC, datetime
from types import TracebackType
from typing import Final, Literal, Self, TypedDict

from litellm.proxy._experimental.mcp_server import db as mcp_db
from litellm.proxy._experimental.mcp_server.db import OAuthCredentialPayload
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Where = dict[str, JsonValue]


class PayloadSummary(TypedDict):
    type: str | None
    access_token: str | None
    refresh_token: str | None
    scopes: list[str] | None
    identity_binding_proof: str | None
    connected_at_present: bool
    expires_in_seconds: int | None


class StoreAnswer(TypedDict):
    refused: bool
    oauth: PayloadSummary | None
    byok: str | None


class DeleteAnswer(TypedDict):
    rows: list[list[str]]


class ItemView(TypedDict):
    user_id: str
    credential_type: str
    expires_at: str | None
    connected_at: str | None
    updated_at: str


class SaltedPair(TypedDict):
    salt_key: str
    user_id: str
    server_id: str
    rows: list[Where]


class GetCredential(SaltedPair):
    function: Literal["get_credential"]


class GetOauthCredential(SaltedPair):
    function: Literal["get_oauth_credential"]


class StoreCredential(SaltedPair):
    function: Literal["store_credential"]
    credential: str


class StoreOauthCredential(SaltedPair):
    function: Literal["store_oauth_credential"]
    access_token: str
    refresh_token: str | None
    expires_in: int | None
    scopes: list[str] | None
    skip_byok_guard: bool
    identity_binding_proof: str | None


class DeleteCredential(TypedDict):
    function: Literal["delete_credential"]
    user_id: str
    server_id: str
    rows: list[Where]


class IsExpired(TypedDict):
    function: Literal["is_expired"]
    credential: OAuthCredentialPayload
    buffer_seconds: int


class GrantState(TypedDict):
    function: Literal["grant_state"]
    credential: OAuthCredentialPayload | None


class ListServerCredentials(TypedDict):
    function: Literal["list_server_credentials"]
    salt_key: str
    server_id: str
    rows: list[Where]


class ListUserOauthCredentials(TypedDict):
    function: Literal["list_user_oauth_credentials"]
    salt_key: str
    user_id: str
    rows: list[Where]


Input = (
    GetCredential
    | GetOauthCredential
    | StoreCredential
    | StoreOauthCredential
    | DeleteCredential
    | IsExpired
    | GrantState
    | ListServerCredentials
    | ListUserOauthCredentials
)
Answer = (
    OAuthCredentialPayload
    | StoreAnswer
    | DeleteAnswer
    | list[ItemView]
    | list[OAuthCredentialPayload]
    | bool
    | str
    | None
)
STRING: Final = TypeAdapter(str)
WHERE: Final = TypeAdapter(Where)
ITEM_VIEW: Final = TypeAdapter(ItemView)


class FakeRow:
    def __init__(self, values: Where) -> None:
        self.user_id = STRING.validate_python(values["user_id"], strict=True)
        self.server_id = STRING.validate_python(values["server_id"], strict=True)
        self.credential_b64 = STRING.validate_python(values["credential_b64"], strict=True)
        stamp = STRING.validate_python(values["updated_at"], strict=True)
        self.updated_at = datetime.fromisoformat(stamp)


def where_matches(row: FakeRow, where: Where) -> bool:
    values: Where = {
        "user_id": row.user_id,
        "server_id": row.server_id,
        "credential_b64": row.credential_b64,
        "updated_at": row.updated_at.isoformat(),
    }
    return all(values[field] == wanted for field, wanted in where.items())


class FakeTable(PrismaWrapper):
    def __init__(self, rows: list[Where]) -> None:
        self.rows = [FakeRow(row) for row in rows]

    def tx(self) -> Self:
        return self

    @override
    def __getattr__(self, name: str) -> Self:
        del name
        return self

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        _kind: type[BaseException] | None,
        _value: BaseException | None,
        _traceback: TracebackType | None,
    ) -> None: ...

    async def find_unique(self, where: Where) -> FakeRow | None:
        key = WHERE.validate_python(where["user_id_server_id"])
        user_id = STRING.validate_python(key["user_id"], strict=True)
        server_id = STRING.validate_python(key["server_id"], strict=True)
        for row in self.rows:
            if row.user_id == user_id and row.server_id == server_id:
                return row
        return None

    async def find_many(self, where: Where | None = None) -> list[FakeRow]:
        if where is None:
            return list(self.rows)
        return [row for row in self.rows if where_matches(row, where)]

    async def upsert(self, where: Where, data: Where) -> FakeRow:
        found = await self.find_unique(where)
        if found is not None:
            update = WHERE.validate_python(data["update"])
            found.credential_b64 = STRING.validate_python(update["credential_b64"], strict=True)
            found.updated_at = datetime.now(UTC)
            return found
        create = WHERE.validate_python(data["create"])
        create["updated_at"] = datetime.now(UTC).isoformat()
        row = FakeRow(create)
        self.rows.append(row)
        return row

    async def delete(self, where: Where) -> FakeRow | None:
        found = await self.find_unique(where)
        if found is not None:
            self.rows.remove(found)
        return found


class FakeClient(PrismaClient):
    def __init__(self, rows: list[Where]) -> None:
        self.fake_table = FakeTable(rows)
        self.db = self.fake_table


def stored_view(client: FakeClient, user_id: str, server_id: str, *, refused: bool) -> StoreAnswer:
    payload = asyncio.run(mcp_db.get_user_oauth_credential(client, user_id, server_id))
    if payload is None:
        byok = asyncio.run(mcp_db.get_user_credential(client, user_id, server_id))
        return StoreAnswer(refused=refused, oauth=None, byok=byok)
    connected = payload.get("connected_at")
    expires = payload.get("expires_at")
    expires_in_seconds: int | None = None
    if connected is not None and expires is not None:
        lifetime = datetime.fromisoformat(expires) - datetime.fromisoformat(connected)
        expires_in_seconds = round(lifetime.total_seconds())
    summary = PayloadSummary(
        type=payload.get("type"),
        access_token=payload.get("access_token"),
        refresh_token=payload.get("refresh_token"),
        scopes=payload.get("scopes"),
        identity_binding_proof=payload.get("identity_binding_proof"),
        connected_at_present=connected is not None,
        expires_in_seconds=expires_in_seconds,
    )
    return StoreAnswer(refused=refused, oauth=summary, byok=None)


def store_credential_call(client: FakeClient, call: StoreCredential) -> StoreAnswer:
    asyncio.run(
        mcp_db.store_user_credential(client, call["user_id"], call["server_id"], call["credential"])
    )
    return stored_view(client, call["user_id"], call["server_id"], refused=False)


def store_oauth_call(client: FakeClient, call: StoreOauthCredential) -> StoreAnswer:
    refused = False
    try:
        asyncio.run(
            mcp_db.store_user_oauth_credential(
                client, call["user_id"], call["server_id"], call["access_token"],
                refresh_token=call["refresh_token"],
                expires_in=call["expires_in"],
                scopes=call["scopes"],
                skip_byok_guard=call["skip_byok_guard"],
                identity_binding_proof=call["identity_binding_proof"],
            )
        )
    except ValueError:
        refused = True
    return stored_view(client, call["user_id"], call["server_id"], refused=refused)


def delete_call(client: FakeClient, call: DeleteCredential) -> DeleteAnswer:
    asyncio.run(mcp_db.delete_user_credential(client, call["user_id"], call["server_id"]))
    return DeleteAnswer(rows=[[row.user_id, row.server_id] for row in client.fake_table.rows])


def run(call: Input) -> Answer:
    answer: Answer = None
    match call["function"]:
        case "get_credential":
            client = FakeClient(call["rows"])
            answer = asyncio.run(
                mcp_db.get_user_credential(client, call["user_id"], call["server_id"])
            )
        case "get_oauth_credential":
            client = FakeClient(call["rows"])
            answer = asyncio.run(
                mcp_db.get_user_oauth_credential(client, call["user_id"], call["server_id"])
            )
        case "store_credential":
            answer = store_credential_call(FakeClient(call["rows"]), call)
        case "store_oauth_credential":
            answer = store_oauth_call(FakeClient(call["rows"]), call)
        case "delete_credential":
            answer = delete_call(FakeClient(call["rows"]), call)
        case "is_expired":
            answer = mcp_db.is_oauth_credential_expired(
                call["credential"], buffer_seconds=call["buffer_seconds"]
            )
        case "grant_state":
            answer = mcp_db.oauth_grant_state(call["credential"])
        case "list_server_credentials":
            client = FakeClient(call["rows"])
            items = asyncio.run(mcp_db.list_server_user_credentials(client, call["server_id"]))
            answer = [ITEM_VIEW.validate_json(item.model_dump_json()) for item in items]
        case "list_user_oauth_credentials":
            client = FakeClient(call["rows"])
            answer = asyncio.run(mcp_db.list_user_oauth_credentials(client, call["user_id"]))
    return answer
