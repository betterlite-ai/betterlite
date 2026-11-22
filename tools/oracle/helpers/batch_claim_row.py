import asyncio
from collections.abc import Mapping, Sequence
from typing import TypedDict

from litellm.proxy._types import SpendLogsPayload
from litellm.proxy.db.db_spend_update_writer import DBSpendUpdateWriter
from litellm.proxy.db.prisma_client import PrismaWrapper
from litellm.proxy.utils import PrismaClient, dequeue_spend_logs
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

ARETRIEVE_BATCH = "aretrieve_batch"
SUCCESS = "success"
DEQUEUE_LIMIT = 1000

TEXT = TypeAdapter(str)
OPTIONAL_TEXT = TypeAdapter(str | None)
NUMBER = TypeAdapter(int | float)
ROW = TypeAdapter(dict[str, JsonValue])


class Payload(SpendLogsPayload):
    messages: str | list[JsonValue] | dict[str, JsonValue] | None
    response: str | list[JsonValue] | dict[str, JsonValue] | None


PAYLOAD = TypeAdapter(Payload)


class Call(TypedDict):
    payload: Payload


class Input(TypedDict):
    disable_spend_logs: bool
    database: bool
    seed: list[Payload]
    calls: list[Call]


class StoredRow:
    def __init__(self, row: Mapping[str, JsonValue]) -> None:
        self.values: dict[str, JsonValue] = dict(row)
        self.request_id: str = TEXT.validate_python(self.values["request_id"])
        self.call_type: str | None = OPTIONAL_TEXT.validate_python(self.values.get("call_type"))
        self.status: str | None = OPTIONAL_TEXT.validate_python(self.values.get("status"))
        self.spend: int | float = NUMBER.validate_python(self.values.get("spend"))


class FakeSpendLogs:
    def __init__(self, seeded: Sequence[Payload]) -> None:
        self.stored: list[StoredRow] = [StoredRow(ROW.validate_python(row)) for row in seeded]

    async def create_many(
        self, *, data: Sequence[Mapping[str, JsonValue]], skip_duplicates: bool
    ) -> int:
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

    async def find_unique(self, *, where: Mapping[str, JsonValue]) -> StoredRow | None:
        request_id = TEXT.validate_python(where["request_id"])
        return next((row for row in self.stored if row.request_id == request_id), None)

    async def update_many(
        self, *, data: Mapping[str, JsonValue], where: Mapping[str, JsonValue]
    ) -> int:
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


# SpendLogsRepository reads db.litellm_spendlogs, so the wrapper hands out the one scripted table.
class FakeDb(PrismaWrapper):
    def __init__(self, table: FakeSpendLogs) -> None:
        self.table = table

    @override
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
        payload: Payload,
        client: PrismaClient | None,
        disabled: bool,
    ) -> bool:
        return await self._record_spend_log(
            payload=payload,
            prisma_client=client,
            disable_spend_logs=disabled,
        )


def row_view(row: Mapping[str, JsonValue]) -> dict[str, JsonValue]:
    view: dict[str, JsonValue] = dict(row)
    spend = row.get("spend")
    if spend is not None:
        view["spend"] = f"{NUMBER.validate_python(spend):.10f}"
    return view


async def record_calls(
    input_value: Input, seed: list[Payload], payloads: list[Payload]
) -> dict[str, JsonValue]:
    table = FakeSpendLogs(seed)
    writer = CaseWriter()
    client = FakeClient(table) if input_value["database"] else None
    calls: list[JsonValue] = []
    for payload in payloads:
        recorded = await writer.record(
            payload=payload,
            client=client,
            disabled=input_value["disable_spend_logs"],
        )
        queued: list[JsonValue] = []
        if client is not None:
            popped = await dequeue_spend_logs(client, DEQUEUE_LIMIT)
            queued = [row_view(ROW.validate_python(row)) for row in popped]
        calls.append({"recorded": recorded, "queued": queued})
    rows: list[JsonValue] = [row_view(row.values) for row in table.stored]
    return {"calls": calls, "rows": rows}


def run(input_value: Input) -> dict[str, JsonValue]:
    seed = [PAYLOAD.validate_python(row) for row in input_value["seed"]]
    payloads = [PAYLOAD.validate_python(call["payload"]) for call in input_value["calls"]]
    return asyncio.run(record_calls(input_value, seed, payloads))
