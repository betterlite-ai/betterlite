import json
from datetime import datetime
from typing import TypeAlias, TypeVar, cast

from litellm.proxy.lens.models import FindingDraft, Lens, LensSettings, RunRequest, Scope, Worker
from litellm.proxy.lens.state import (
    can_access,
    claim_job,
    merge_finding,
    queue_job,
    renew_budget,
    snapshot_finding,
)
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Record = TypeVar("Record", bound=BaseModel)
# betterlite writes money as a JSON float, while Pydantic leaves an unvalidated int default an int
MONEY: frozenset[str] = frozenset({"cost", "monthly_budget", "sample_percent", "spent"})


def parse(model: type[Record], value: Json) -> Record:
    return model.model_validate_json(json.dumps(value))


def floats(value: Json) -> Json:
    if isinstance(value, list):
        return [floats(item) for item in value]
    if isinstance(value, dict):
        return {
            key: float(item) if key in MONEY and isinstance(item, int) else floats(item)
            for key, item in value.items()
        }
    return value


def dump(record: BaseModel) -> Json:
    return floats(cast("Json", json.loads(record.model_dump_json())))


def moment(value: Json) -> datetime:
    if not isinstance(value, str):
        raise ValueError("now must be an ISO 8601 string")
    return datetime.fromisoformat(value)


def finding_arguments(call: Json) -> tuple[Lens, FindingDraft, int, datetime]:
    match call:
        case {"lens": lens, "draft": draft, "revision": int(revision), "now": now}:
            return parse(Lens, lens), parse(FindingDraft, draft), revision, moment(now)
        case _:
            raise ValueError("a finding call takes lens, draft, revision and now")


def run(input_value: dict[str, Json]) -> Json:
    result: Json
    match input_value:
        case {"can_access": {"viewer": viewer, "target": target}}:
            result = can_access(parse(Scope, viewer), parse(Scope, target))
        case {"queue_job": {"lens": lens, "now": now, "job_id": str(job_id), "request": request}}:
            body = parse(RunRequest, request)
            queued = queue_job(
                parse(Lens, lens), moment(now), job_id, body.lookback_hours, body.settings
            )
            result = dump(queued)
        case {"claim_job": {"lens": lens, "worker": worker, "now": now}}:
            result = dump(claim_job(parse(Lens, lens), parse(Worker, worker), moment(now)))
        case {"renew_budget": {"lens": lens, "now": now}}:
            result = dump(renew_budget(parse(Lens, lens), moment(now)))
        case {"merge_finding": call}:
            result = dump(merge_finding(*finding_arguments(call)))
        case {"snapshot_finding": call}:
            result = dump(snapshot_finding(*finding_arguments(call)))
        case {"decode_settings": {"json": value}}:
            result = dump(parse(LensSettings, value))
        case {"analysis_checks": {"settings": settings}}:
            result = [dump(check) for check in parse(LensSettings, settings).analysis_checks]
        case _:
            raise ValueError("lens_state input names no known operation")
    return result
