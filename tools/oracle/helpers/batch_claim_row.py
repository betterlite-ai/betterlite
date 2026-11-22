import asyncio
from collections.abc import Mapping, Sequence
from typing import TypedDict

from litellm.proxy._types import SpendLogsPayload
from litellm.proxy.db.db_spend_update_writer import DBSpendUpdateWriter
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, dequeue_spend_logs
from pydantic import JsonValue, TypeAdapter

ARETRIEVE_BATCH = "aretrieve_batch"
SUCCESS = "success"
DEQUEUE_LIMIT = 1000

TEXT = TypeAdapter(str)
OPTIONAL_TEXT = TypeAdapter(str | None)
NUMBER = TypeAdapter(int | float)
VALUE = TypeAdapter(JsonValue)


class Call(TypedDict):
    payload: SpendLogsPayload


class Input(TypedDict):
    disable_spend_logs: bool
    database: bool
    seed: list[SpendLogsPayload]
    calls: list[Call]


class StoredRow:
    def __init__(self, row: dict[str, object]) -> None:
        self.values = row
        self.request_id = TEXT.validate_python(row["request_id"])
        self.call_type = OPTIONAL_TEXT.validate_python(row.get("call_type"))
        self.status = OPTIONAL_TEXT.validate_python(row.get("status"))
        self.spend = NUMBER.validate_python(row.get("spend"))


class FakeSpendLogs:
    def __init__(self, seeded: Sequence[Mapping[str, object]]) -> None:
        self.stored: list[StoredRow] = [StoredRow(dict(row)) for row in seeded]

    async def create_many(self, *, data: list[dict[str, object]], skip_duplicates: bool) -> int:
        inserted = 0
        for row in data:
            candidate = StoredRow(row)
            if skip_duplicates and any(
                existing.request_id == candidate.request_id for existing in self.stored
            ):
                continue
            self.stored.append(candidate)
            inserted += 1
        return inserted

    async def find_unique(self, *, where: dict[str, object]) -> StoredRow | None:
        request_id = TEXT.validate_python(where["request_id"])
        return next((row for row in self.stored if row.request_id == request_id), None)

    async def update_many(self, *, data: dict[str, object], where: dict[str, object]) -> int:
        request_id = TEXT.validate_python(where["request_id"])
        matching = [
            row
            for row in self.stored
            if row.request_id == request_id
            and row.call_type == ARETRIEVE_BATCH
            and row.status == SUCCESS
            and row.spend == 0
        ]
        for row in matching:
            for field, value in data.items():
                row.values[field] = value
            row.spend = NUMBER.validate_python(row.values.get("spend"))
        return len(matching)


# SpendLogsRepository reads prisma_client.db.litellm_spendlogs, so the db wrapper hands out the one scripted table.
class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeSpendLogs) -> None:
        self.table = table

    def __getattr__(self, name: str) -> FakeSpendLogs:
        if name == "litellm_spendlogs":
            return self.table
        raise AttributeError(name)


class FakeClient(PrismaClient):
    def __init__(self, table: FakeSpendLogs) -> None:
        self.db = FakeDb(table)


class CaseWriter(DBSpendUpdateWriter):
    async def record(
        self,
        *,
        payload: SpendLogsPayload,
        client: PrismaClient | None,
        disabled: bool,
    ) -> bool:
        return await self._record_spend_log(
            payload=payload,
            prisma_client=client,
            disable_spend_logs=disabled,
        )


def row_view(row: dict[str, object]) -> dict[str, JsonValue]:
    view: dict[str, JsonValue] = {}
    for field, value in row.items():
        view[field] = VALUE.validate_python(value)
    spend = row.get("spend")
    if spend is not None:
        view["spend"] = f"{NUMBER.validate_python(spend):.10f}"
    return view


async def record_calls(input_value: Input) -> dict[str, JsonValue]:
    table = FakeSpendLogs(input_value["seed"])
    writer = CaseWriter()
    client = None if input_value["database"] is False else FakeClient(table)
    calls: list[JsonValue] = []
    for call in input_value["calls"]:
        recorded = await writer.record(
            payload=call["payload"],
            client=client,
            disabled=input_value["disable_spend_logs"],
        )
        queued: list[JsonValue] = []
        if client is not None:
            queued = [row_view(row) for row in await dequeue_spend_logs(client, DEQUEUE_LIMIT)]
        calls.append({"recorded": recorded, "queued": queued})
    rows: list[JsonValue] = [row_view(row.values) for row in table.stored]
    return {"calls": calls, "rows": rows}


def run(input_value: Input) -> dict[str, JsonValue]:
    return asyncio.run(record_calls(input_value))
