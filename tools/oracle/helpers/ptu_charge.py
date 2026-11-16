from datetime import UTC, date, datetime, timedelta
from types import SimpleNamespace
from typing import Final, Literal, TypedDict

from litellm.constants import PTU_PRUNE_SKEW_GRACE_SECONDS, PTU_SENTINEL_API_KEY
from litellm.proxy.spend_tracking.ptu_flat_cost_rollup import (
    _PRUNE_ID_CHUNK_SIZE,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L60
    PTUModel,
    _active_hours_on_day,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L186
    _aggregate_charges,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L217
    _backfill_window,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L497
    _compute_daily_flat_cost,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L197
    _config_deployments,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L348
    _decode_model_info,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L112
    _lapsed_models,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L479
    _parse_ptu_model,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L163
    _prune_filter,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/spend_tracking/ptu_flat_cost_rollup.py#L745
)
from pydantic import JsonValue, TypeAdapter

EPOCH: Final = datetime(1970, 1, 1, tzinfo=UTC)


class ModelInput(TypedDict):
    model_id: str
    model_name: str
    team_id: str
    ptu_count: int
    cost_per_ptu_per_hour: str
    effective_from_micros: int
    effective_to_micros: int | None


class EntryInput(TypedDict):
    model_name: str
    model_info: JsonValue


class DecodeStep(TypedDict):
    kind: Literal["decode"]
    raw: JsonValue


class ParseStep(TypedDict):
    kind: Literal["parse"]
    model_id: str
    model_name: str
    model_info: JsonValue


class HoursStep(TypedDict):
    kind: Literal["hours"]
    model: ModelInput
    day: str


class CostStep(TypedDict):
    kind: Literal["cost"]
    model: ModelInput
    day: str


class AggregateStep(TypedDict):
    kind: Literal["aggregate"]
    models: list[ModelInput]
    day: str


class ConfigStep(TypedDict):
    kind: Literal["config"]
    entries: list[EntryInput]
    owned_by_db: list[str]


class LapsedStep(TypedDict):
    kind: Literal["lapsed"]
    models: list[ModelInput]
    now_micros: int


class BackfillStep(TypedDict):
    kind: Literal["backfill"]
    models: list[ModelInput]
    end: str


class PruneStep(TypedDict):
    kind: Literal["prune"]
    date: str
    run_started_micros: int
    scanned_ids: list[str]


Step = (
    DecodeStep
    | ParseStep
    | HoursStep
    | CostStep
    | AggregateStep
    | ConfigStep
    | LapsedStep
    | BackfillStep
    | PruneStep
)


class Input(TypedDict):
    steps: dict[str, Step]


def instant_from_micros(micros: int) -> datetime:
    delta = timedelta(microseconds=micros)
    return EPOCH + delta


def instant_micros(value: datetime) -> int:
    delta = value - EPOCH
    return delta.days * 86_400_000_000 + delta.seconds * 1_000_000 + delta.microseconds


def optional_instant(micros: int | None) -> datetime | None:
    if micros is None:
        return None
    return instant_from_micros(micros)


def optional_micros(value: datetime | None) -> int | None:
    if value is None:
        return None
    return instant_micros(value)


def ptu_model(model: ModelInput) -> PTUModel:
    return PTUModel(
        model_id=model["model_id"],
        model_name=model["model_name"],
        team_id=model["team_id"],
        ptu_count=model["ptu_count"],
        cost_per_ptu_per_hour=float(model["cost_per_ptu_per_hour"]),
        effective_from=instant_from_micros(model["effective_from_micros"]),
        effective_to=optional_instant(model["effective_to_micros"]),
    )


def decode_step(step: DecodeStep) -> JsonValue:
    decoded = _decode_model_info(step["raw"])
    if decoded is None:
        return None
    return TypeAdapter(JsonValue).validate_python(decoded)


def parse_step(step: ParseStep) -> JsonValue:
    row = SimpleNamespace(
        model_id=step["model_id"],
        model_name=step["model_name"],
        model_info=step["model_info"],
    )
    parsed = _parse_ptu_model(row)
    if parsed is None:
        return None
    record: dict[str, JsonValue] = {
        "model_id": parsed.model_id,
        "model_name": parsed.model_name,
        "team_id": parsed.team_id,
        "ptu_count": parsed.ptu_count,
        "cost_per_ptu_per_hour": parsed.cost_per_ptu_per_hour,
        "effective_from_micros": optional_micros(parsed.effective_from),
        "effective_to_micros": optional_micros(parsed.effective_to),
    }
    return record


def hours_step(step: HoursStep) -> float:
    model = ptu_model(step["model"])
    day = date.fromisoformat(step["day"])
    return _active_hours_on_day(model, day)


def cost_step(step: CostStep) -> float:
    model = ptu_model(step["model"])
    day = date.fromisoformat(step["day"])
    return _compute_daily_flat_cost(model, day)


def aggregate_step(step: AggregateStep) -> list[JsonValue]:
    models = tuple(ptu_model(model) for model in step["models"])
    charges = _aggregate_charges(models, date.fromisoformat(step["day"]))
    records: list[JsonValue] = []
    for charge in charges:
        record: dict[str, JsonValue] = {
            "team_id": charge.team_id,
            "model_id": charge.model_id,
            "model_name": charge.model_name,
            "flat_cost": charge.flat_cost,
        }
        records.append(record)
    return records


def config_step(step: ConfigStep) -> list[JsonValue]:
    router = SimpleNamespace(model_list=step["entries"])
    deployments = _config_deployments(router, owned_by_db=frozenset(step["owned_by_db"]))
    records: list[JsonValue] = []
    for deployment in deployments:
        record: dict[str, JsonValue] = {
            "model_id": deployment.model_id,
            "model_name": deployment.model_name,
        }
        records.append(record)
    return records


def lapsed_step(step: LapsedStep) -> list[JsonValue]:
    models = tuple(ptu_model(model) for model in step["models"])
    names = _lapsed_models(models, instant_from_micros(step["now_micros"]))
    return [*names]


def backfill_step(step: BackfillStep) -> list[JsonValue]:
    models = tuple(ptu_model(model) for model in step["models"])
    days = _backfill_window(models, date.fromisoformat(step["end"]))
    records: list[JsonValue] = []
    for day in days:
        label = day.isoformat()
        records.append(label)
    return records


def prune_step(step: PruneStep) -> list[JsonValue]:
    started = instant_from_micros(step["run_started_micros"])
    cutoff = started - timedelta(seconds=PTU_PRUNE_SKEW_GRACE_SECONDS)
    ordered = sorted(set(step["scanned_ids"]))
    records: list[JsonValue] = []
    for start in range(0, len(ordered), _PRUNE_ID_CHUNK_SIZE):
        chunk = ordered[start : start + _PRUNE_ID_CHUNK_SIZE]
        predicate = _prune_filter(date_str=step["date"], cutoff=cutoff, chunk=tuple(chunk))
        expected = {
            "date": step["date"],
            "api_key": PTU_SENTINEL_API_KEY,
            "updated_at": {"lt": cutoff},
            "model": {"in": tuple(chunk)},
        }
        if dict(predicate) != expected:
            raise ValueError("the prune predicate no longer matches the sentinel delete")
        chunk_ids: list[JsonValue] = [*chunk]
        record: dict[str, JsonValue] = {
            "date": step["date"],
            "cutoff_micros": instant_micros(cutoff),
            "api_key": PTU_SENTINEL_API_KEY,
            "models": chunk_ids,
        }
        records.append(record)
    return records


def step_value(step: Step) -> JsonValue:
    if step["kind"] == "decode":
        return decode_step(step)
    if step["kind"] == "parse":
        return parse_step(step)
    if step["kind"] == "hours":
        return hours_step(step)
    if step["kind"] == "cost":
        return cost_step(step)
    if step["kind"] == "aggregate":
        return aggregate_step(step)
    if step["kind"] == "config":
        return config_step(step)
    if step["kind"] == "lapsed":
        return lapsed_step(step)
    if step["kind"] == "backfill":
        return backfill_step(step)
    if step["kind"] == "prune":
        return prune_step(step)
    raise ValueError("unknown ptu_charge step")


def run(input_value: Input) -> dict[str, JsonValue]:
    answers: dict[str, JsonValue] = {}
    for name, step in input_value["steps"].items():
        answers[name] = step_value(step)
    return answers
