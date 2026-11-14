from typing import Literal, NotRequired, TypeAlias, TypedDict

from litellm.proxy.lens.models import Job, Lens, Worker
from pydantic import JsonValue, TypeAdapter

# LiteLLM's Database protocol takes object arguments, so a narrower type breaks it
SqlArgument: TypeAlias = object


class LensSeed(TypedDict):
    version: int
    data: dict[str, JsonValue]


class RunSeed(TypedDict):
    lens_id: str
    data: dict[str, JsonValue]


class WorkerSeed(TypedDict):
    token_hash: str
    data: dict[str, JsonValue]


class Seed(TypedDict):
    lenses: NotRequired[dict[str, LensSeed]]
    runs: NotRequired[dict[str, RunSeed]]
    workers: NotRequired[dict[str, WorkerSeed]]


class Transform(TypedDict):
    kind: Literal["unchanged", "set_revision", "drop_job"]
    revision: NotRequired[int]
    job_id: NotRequired[str]


class Call(TypedDict):
    operation: Literal[
        "update",
        "jobs",
        "job",
        "save_worker",
        "set_worker_billing",
        "revoke_worker",
        "heartbeat",
    ]
    lens_id: NotRequired[str]
    attempts: NotRequired[int]
    changed_only: NotRequired[bool]
    transform: NotRequired[Transform]
    conflicts: NotRequired[int]
    offset: NotRequired[int]
    job_id: NotRequired[str]
    token_hash: NotRequired[str | None]
    worker: NotRequired[dict[str, JsonValue]]
    worker_id: NotRequired[str]
    key_id: NotRequired[str]
    now: NotRequired[str]


class Input(TypedDict):
    seed: NotRequired[Seed]
    call: Call


class Output(TypedDict):
    result: JsonValue
    transform_calls: int
    lenses: list[dict[str, JsonValue]]
    runs: list[dict[str, JsonValue]]
    workers: list[dict[str, JsonValue]]


class StoredLens(TypedDict):
    version: int
    lens: Lens


class StoredRun(TypedDict):
    lens_id: str
    job: Job


class StoredWorker(TypedDict):
    token_hash: str
    data: dict[str, JsonValue]


def dump(model: Lens | Job | Worker) -> JsonValue:
    text = model.model_dump_json()
    return TypeAdapter(JsonValue).validate_json(text)


def dump_mapping(model: Lens | Job | Worker) -> dict[str, JsonValue]:
    text = model.model_dump_json()
    return TypeAdapter(dict[str, JsonValue]).validate_json(text)


def count_row(count: int) -> dict[str, JsonValue]:
    row: dict[str, JsonValue] = {"data": count}
    return row


def revised(lens: Lens, field: str, value: JsonValue) -> Lens:
    data = dump_mapping(lens)
    data[field] = value
    return Lens.model_validate(data)
