from datetime import UTC, datetime
from decimal import Decimal
from typing import NotRequired, TypedDict

from litellm.proxy.db.spend_log_batching import (
    SpendLogRow,
    spend_log_queue_within_budget,
    spend_log_row_bytes,
    spend_log_write_batches,
)
from litellm.proxy.db.spend_log_tool_index import build_tool_usage_transaction
from pydantic import JsonValue


class TransactionInput(TypedDict):
    request_id: str
    start_time_iso: str
    mcp_namespaced_tool_name: str | None
    spend: float
    total_tokens: int
    completion_response: dict[str, JsonValue] | None
    realtime_tool_calls: list[dict[str, JsonValue]] | None


class RowInput(TypedDict):
    request_id: str
    tool_name: str
    start_time: str
    refused: bool


class Input(TypedDict):
    transaction: NotRequired[TransactionInput | None]
    rows: NotRequired[list[RowInput] | None]
    max_bytes: NotRequired[int | None]
    max_rows: NotRequired[int | None]
    queued_bytes: NotRequired[int | None]


class TimeOutput(TypedDict):
    epoch_seconds: int
    micros: int


class TransactionOutput(TypedDict):
    request_id: str
    date: str
    start_time: TimeOutput
    tool_names: list[str]
    spend: str
    total_tokens: int


class TrimmedOutput(TypedDict):
    start: int
    queued_bytes: int


class Output(TypedDict):
    transaction: TransactionOutput | None
    sizes: list[int]
    batches: list[list[int]] | None
    trimmed: TrimmedOutput | None


def _decimal_str(value: float) -> str:
    normalized = Decimal(repr(value)).normalize()
    return format(normalized, "f")


def _spend_row(row: RowInput) -> SpendLogRow:
    moment = datetime.fromisoformat(row["start_time"])
    if moment.tzinfo is None:
        moment = moment.replace(tzinfo=UTC)
    return {
        "request_id": row["request_id"],
        "tool_name": row["tool_name"],
        "start_time": moment,
    }


def _refused_row() -> SpendLogRow:
    circular: dict[str, SpendLogRow] = {}
    circular["refused"] = circular
    return circular


def _transaction(input_value: TransactionInput | None) -> TransactionOutput | None:
    if input_value is None:
        return None
    transaction = build_tool_usage_transaction(
        request_id=input_value["request_id"],
        start_time_iso=input_value["start_time_iso"],
        mcp_namespaced_tool_name=input_value["mcp_namespaced_tool_name"],
        spend=input_value["spend"],
        total_tokens=input_value["total_tokens"],
        completion_response=input_value["completion_response"],
        realtime_tool_calls=input_value["realtime_tool_calls"],
    )
    if transaction is None:
        return None
    return {
        "request_id": transaction.request_id,
        "date": transaction.date,
        "start_time": {
            "epoch_seconds": int(transaction.start_time.timestamp()),
            "micros": transaction.start_time.microsecond,
        },
        "tool_names": list(transaction.tool_names),
        "spend": _decimal_str(transaction.spend),
        "total_tokens": transaction.total_tokens,
    }


def _batches(rows: list[SpendLogRow], max_bytes: int, max_rows: int) -> list[list[int]]:
    batches: list[list[int]] = []
    start = 0
    for batch in spend_log_write_batches(rows, max_bytes, max_rows):
        end = start + len(batch)
        batches.append(list(range(start, end)))
        start = end
    return batches


def _trimmed(
    rows: list[SpendLogRow],
    queued_bytes: int | None,
    max_bytes: int | None,
) -> TrimmedOutput | None:
    if queued_bytes is None or max_bytes is None:
        return None
    kept, total = spend_log_queue_within_budget(rows, queued_bytes, max_bytes)
    return {"start": len(rows) - len(kept), "queued_bytes": total}


def run(input_value: Input) -> Output:
    rows: list[SpendLogRow] = []
    rows_input = input_value.get("rows")
    if rows_input is not None:
        rows = [_refused_row() if row["refused"] else _spend_row(row) for row in rows_input]
    max_bytes = input_value.get("max_bytes")
    max_rows = input_value.get("max_rows")
    batches: list[list[int]] | None = None
    if max_bytes is not None and max_rows is not None:
        batches = _batches(rows, max_bytes, max_rows)
    return {
        "transaction": _transaction(input_value.get("transaction")),
        "sizes": [spend_log_row_bytes(row) for row in rows],
        "batches": batches,
        "trimmed": _trimmed(rows, input_value.get("queued_bytes"), max_bytes),
    }
