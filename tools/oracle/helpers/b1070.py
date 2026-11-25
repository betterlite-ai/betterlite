import asyncio
import sys
import types
from typing import Final, Literal, TypedDict

import litellm
from fastapi import HTTPException
from litellm.proxy._types import (
    LitellmUserRoles,
    ResponseLiteLLM_ManagedVectorStore,
    UserAPIKeyAuth,
)
from litellm.proxy.vector_store_endpoints.management_endpoints import (
    get_vector_store_info,
    list_vector_stores,
)
from litellm.types.vector_stores import (
    LiteLLM_ManagedVectorStore,
    LiteLLM_ManagedVectorStoreListResponse,
    VectorStoreInfoRequest,
)
from litellm.vector_stores.vector_store_registry import VectorStoreRegistry
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
LIST_ADAPTER: Final[TypeAdapter[LiteLLM_ManagedVectorStoreListResponse]] = TypeAdapter(
    LiteLLM_ManagedVectorStoreListResponse
)
INFO_ADAPTER: Final[TypeAdapter[ResponseLiteLLM_ManagedVectorStore]] = TypeAdapter(
    ResponseLiteLLM_ManagedVectorStore
)


class Caller(TypedDict):
    user_id: str
    user_role: str
    team_id: str | None


class ListCall(TypedDict):
    route: Literal["list"]
    page: int
    page_size: int
    alias: bool
    database: list[LiteLLM_ManagedVectorStore] | None
    registry: list[LiteLLM_ManagedVectorStore] | None
    caller: Caller
    disabled: bool


class InfoCall(TypedDict):
    route: Literal["info"]
    vector_store_id: str
    database: list[LiteLLM_ManagedVectorStore] | None
    registry: list[LiteLLM_ManagedVectorStore] | None
    caller: Caller
    disabled: bool


class Input(TypedDict):
    calls: list[ListCall | InfoCall]


class FakeRow:
    def __init__(self, row: dict[str, JsonValue]) -> None:
        self._row = row

    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self._row)


class FakeTable:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self._rows = rows

    async def find_many(self, **_: object) -> list[dict[str, JsonValue]]:
        return self._rows

    async def find_unique(self, where: dict[str, str]) -> FakeRow | None:
        for row in self._rows:
            if row.get("vector_store_id") == where["vector_store_id"]:
                return FakeRow(row)
        return None


class FakeDb:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.litellm_managedvectorstorestable = FakeTable(rows)
        self.litellm_managedvectorstoreindextable = FakeTable(rows)


class FakePrisma:
    def __init__(self, rows: list[dict[str, JsonValue]]) -> None:
        self.db = FakeDb(rows)
        self.writer_db = FakeDb(rows)


PROXY_MODULE: Final = "litellm.proxy.proxy_server"


class ProxyServer(types.ModuleType):
    def __init__(self, name: str) -> None:
        super().__init__(name)
        self.prisma_client: object = None
        self.general_settings: dict[str, JsonValue] = {}
        self.user_api_key_cache: object = None


PROXY: Final = ProxyServer(PROXY_MODULE)
sys.modules[PROXY_MODULE] = PROXY


def configure(call: ListCall | InfoCall) -> UserAPIKeyAuth:
    rows = call["database"]
    PROXY.prisma_client = None if rows is None else FakePrisma(rows)
    PROXY.general_settings = {
        "disable_vector_stores_for_internal_users": call["disabled"]
    }
    registry = call["registry"]
    litellm.vector_store_registry = (
        None if registry is None else VectorStoreRegistry(vector_stores=registry)
    )
    caller = call["caller"]
    return UserAPIKeyAuth(
        user_id=caller["user_id"],
        user_role=LitellmUserRoles(caller["user_role"]),
        team_id=caller["team_id"],
    )


def error_answer(error: HTTPException) -> JsonValue:
    return {"status": error.status_code, "body": {"detail": error.detail}}


async def answer_list(call: ListCall) -> JsonValue:
    key = configure(call)
    try:
        listed = await list_vector_stores(
            user_api_key_dict=key,
            page=call["page"],
            page_size=call["page_size"],
        )
    except HTTPException as error:
        return error_answer(error)
    return JSON.validate_json(
        LIST_ADAPTER.dump_json(LIST_ADAPTER.validate_python(listed))
    )


async def answer_info(call: InfoCall) -> JsonValue:
    key = configure(call)
    try:
        info = await get_vector_store_info(
            data=VectorStoreInfoRequest(vector_store_id=call["vector_store_id"]),
            user_api_key_dict=key,
        )
    except HTTPException as error:
        return error_answer(error)
    return JSON.validate_json(
        INFO_ADAPTER.dump_json(INFO_ADAPTER.validate_python(info))
    )


async def answer_all(input_value: Input) -> list[JsonValue]:
    answers: list[JsonValue] = []
    for call in input_value["calls"]:
        if call["route"] == "list":
            answers.append(await answer_list(call))
        else:
            answers.append(await answer_info(call))
    return answers


def run(input_value: Input) -> JsonValue:
    return JSON.validate_python(asyncio.run(answer_all(input_value)))
