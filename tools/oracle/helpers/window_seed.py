from datetime import datetime
from typing import Literal, TypeAlias, TypedDict

from litellm.proxy.db.budget_window_spend_writer import WindowSeedTotals, _exclusion_upper_bound, _seed_base, _upsert_params  # noqa: E501  # isort: skip  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/budget_window_spend_writer.py#L167-L258
from litellm.proxy.db.db_transaction_queue.window_spend_update_queue import WindowSpendTransaction


class SeedBaseStep(TypedDict):
    kind: Literal["seed_base"]
    name: str
    total: float
    before_batch: float
    batch_spend: float


class ExclusionStep(TypedDict):
    kind: Literal["exclusion_upper_bound"]
    name: str
    started_at: str


class UpsertParamsStep(TypedDict):
    kind: Literal["upsert_params"]
    name: str
    transaction: WindowSpendTransaction
    seed_base: float
    now: str


Step: TypeAlias = SeedBaseStep | ExclusionStep | UpsertParamsStep


class Input(TypedDict):
    steps: list[Step]


class Params(TypedDict):
    entity_type: str
    entity_id: str
    window_duration: str
    window_start: str
    spend: str
    increment: str
    now: str


Result: TypeAlias = str | Params


def seed_step(step: SeedBaseStep) -> str:
    totals = WindowSeedTotals(total=step["total"], before_batch=step["before_batch"])
    return f"{_seed_base(totals=totals, batch_spend=step['batch_spend']):.6f}"


def exclusion_step(step: ExclusionStep) -> str:
    return _exclusion_upper_bound(datetime.fromisoformat(step["started_at"])).isoformat()


def params_step(step: UpsertParamsStep) -> Params:
    entity_type, entity_id, window_duration, window_start, spend, increment, now = _upsert_params(
        transaction=step["transaction"],
        seed_base=step["seed_base"],
        now=datetime.fromisoformat(step["now"]),
    )
    return Params(
        entity_type=entity_type,
        entity_id=entity_id,
        window_duration=window_duration,
        window_start=window_start.isoformat(),
        spend=f"{spend:.6f}",
        increment=f"{increment:.6f}",
        now=now.isoformat(),
    )


def run(input_value: Input) -> dict[str, Result]:
    results: dict[str, Result] = {}
    for step in input_value["steps"]:
        if step["kind"] == "seed_base":
            results[step["name"]] = seed_step(step)
        elif step["kind"] == "exclusion_upper_bound":
            results[step["name"]] = exclusion_step(step)
        else:
            results[step["name"]] = params_step(step)
    return results
