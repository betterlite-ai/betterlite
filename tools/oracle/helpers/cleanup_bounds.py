import time
from datetime import datetime
from typing import TypedDict

from litellm.proxy.db.db_transaction_queue.spend_log_cleanup import (
    SpendLogCleanup,
    _cutoff_cast,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L39
    _cutoff_text,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L43
)
from pydantic import JsonValue


class GroupInput(TypedDict):
    overall_ms: int
    groups: int


class Input(TypedDict):
    general_settings: dict[str, JsonValue]
    retention: list[str]
    now_ms: int
    deadlines_ms: list[int]
    groups: list[GroupInput]
    timestamps: list[str]
    days: list[str]


class Output(TypedDict):
    bounds: list[int]
    retention: list[int | None]
    timeouts_ms: list[int]
    group_timeouts_ms: list[int]
    cutoffs: list[list[str]]


class FixedClock:
    def __init__(self, seconds: float) -> None:
        self.seconds = seconds

    def __call__(self) -> float:
        return self.seconds


def deadline_timeouts(cleanup: SpendLogCleanup, input_value: Input) -> tuple[list[int], list[int]]:
    direct: list[int] = []
    shared: list[int] = []
    saved = time.monotonic
    time.monotonic = FixedClock(input_value["now_ms"] / 1000)
    try:
        for deadline_ms in input_value["deadlines_ms"]:
            timeout = cleanup._timeout_ms(deadline_ms / 1000)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L243
            direct.append(timeout)
        for group in input_value["groups"]:
            overall = group["overall_ms"] / 1000
            share = SpendLogCleanup._group_deadline(overall, group["groups"])  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L261
            timeout = cleanup._timeout_ms(share)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L243
            shared.append(timeout)
    finally:
        time.monotonic = saved
    return direct, shared


def run(input_value: Input) -> Output:
    cleanup = SpendLogCleanup.__new__(SpendLogCleanup)
    cleanup.general_settings = input_value["general_settings"]
    cleanup._refresh_bounds()  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L146
    retention: list[int | None] = []
    for name in input_value["retention"]:
        seconds = cleanup._retention_seconds_for(name)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/db_transaction_queue/spend_log_cleanup.py#L211
        retention.append(seconds)
    direct, shared = deadline_timeouts(cleanup, input_value)
    cutoffs: list[datetime | str] = [
        datetime.fromisoformat(moment) for moment in input_value["timestamps"]
    ]
    cutoffs.extend(input_value["days"])
    return {
        "bounds": [
            cleanup.batch_size,
            cleanup.max_batches,
            int(cleanup.run_budget_seconds * 1000),
            int(cleanup.batch_timeout_seconds * 1000),
        ],
        "retention": retention,
        "timeouts_ms": direct,
        "group_timeouts_ms": shared,
        "cutoffs": [[_cutoff_cast(cutoff), _cutoff_text(cutoff)] for cutoff in cutoffs],
    }
