from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.db import baseline_accounting as accounting
from litellm.proxy.spend_tracking.baseline_accounting import BaselineEstimate, BaselineHistory
from pydantic import JsonValue, TypeAdapter

serialized, changed, project_group, StoredRecord = accounting._serialized, accounting._change, accounting._project_group, accounting._StoredRecord  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/db/baseline_accounting.py#L313-L365

_JSON: Final = TypeAdapter(JsonValue)
_OBJECT: Final = TypeAdapter(dict[str, JsonValue])
_CHANGES: Final = TypeAdapter(list[JsonValue])
_ESTIMATE: Final = TypeAdapter(BaselineEstimate)
_HISTORY: Final = TypeAdapter(BaselineHistory)
_ENTITY: Final = TypeAdapter(Literal["user", "team", "org", "tag", "end_user", "agent"])


class RecordInput(TypedDict):
    function: Literal["validate", "canonical"]
    record: str


class PublicationInput(TypedDict):
    function: Literal["publication"]
    record: str
    estimate: str
    first_at: float


class ChangeInput(TypedDict):
    function: Literal["change"]
    record: str
    old: str | None
    new: str


class StoredInput(TypedDict):
    data: str
    publication: str | None
    conflicted: bool
    started_at: float


class ProjectInput(TypedDict):
    function: Literal["project"]
    history: str
    stored: list[StoredInput]


class AdjustmentInput(TypedDict):
    function: Literal["adjustment"]
    record: str
    entity: str
    entity_id: str | None
    savings_delta: float
    request_id: str


Input: TypeAlias = RecordInput | PublicationInput | ChangeInput | ProjectInput | AdjustmentInput


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "validate":
        accounting.BaselineAccountingRecord.model_validate_json(input_value["record"])
        return {"accepted": True}
    if input_value["function"] == "canonical":
        record = accounting.BaselineAccountingRecord.model_validate_json(input_value["record"])
        return serialized(record)
    if input_value["function"] == "publication":
        record = accounting.BaselineAccountingRecord.model_validate_json(input_value["record"])
        estimate = _ESTIMATE.validate_json(input_value["estimate"])
        published = accounting.baseline_publication(record, estimate, input_value["first_at"])
        return _JSON.validate_json(published.model_dump_json())
    if input_value["function"] == "change":
        record = accounting.BaselineAccountingRecord.model_validate_json(input_value["record"])
        old = input_value["old"]
        previous = (
            None if old is None else accounting.BaselinePublication.model_validate_json(old)
        )
        current = accounting.BaselinePublication.model_validate_json(input_value["new"])
        return _JSON.validate_json(changed(record, previous, current).model_dump_json())
    if input_value["function"] == "project":
        history = _HISTORY.validate_json(input_value["history"])
        stored = [StoredRecord.model_validate(item) for item in input_value["stored"]]
        _, changes = project_group((history, ()), stored)
        return _CHANGES.validate_json(f"[{','.join(value.model_dump_json() for value in changes)}]")
    record = accounting.BaselineAccountingRecord.model_validate_json(input_value["record"])
    daily = record.daily
    if daily is None:
        raise ValueError("record has no daily attribution")
    target = accounting.DailyBaselineTarget(
        entity=_ENTITY.validate_python(input_value["entity"]),
        entity_id=input_value["entity_id"],
    )
    row = daily.adjustment(target, input_value["savings_delta"], input_value["request_id"])
    return _OBJECT.validate_python(dict(row))
