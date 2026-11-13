import builtins
from typing import Final, TypedDict

from litellm.models.verification_token import LiteLLM_VerificationToken
from litellm.repositories.base_repository import DbRecord
from litellm.repositories.verification_token_repository import VerificationTokenRepository
from pydantic import JsonValue, TypeAdapter


class RecordInput(TypedDict, total=False):
    model_dump: builtins.dict[str, JsonValue]
    dict: builtins.dict[str, JsonValue]
    mapping: builtins.dict[str, JsonValue]
    pairs: list[tuple[str, JsonValue]]


class Case(TypedDict):
    records: list[RecordInput | None]


class Input(TypedDict):
    cases: dict[str, Case]


class DumpRecord(dict[str, JsonValue]):
    def model_dump(self) -> dict[str, JsonValue]:
        return dict(self)


class DictRecord(dict[str, JsonValue]):
    def dict(self) -> dict[str, JsonValue]:
        return dict(self)


class Probe(VerificationTokenRepository):
    def convert(self, record: DbRecord | None) -> LiteLLM_VerificationToken | None:
        return self._to_model(record)


INVALID: Final[dict[str, JsonValue]] = {"invalid": True}
JSON: Final = TypeAdapter(JsonValue)


def record(value: RecordInput | None) -> DbRecord | None:
    if value is None:
        return None
    dumped = value.get("model_dump")
    if dumped is not None:
        return DumpRecord(dumped)
    direct = value.get("dict")
    if direct is not None:
        return DictRecord(direct)
    mapping = value.get("mapping")
    if mapping is not None:
        return mapping
    pairs = value.get("pairs")
    if pairs is not None:
        return pairs
    empty: dict[str, JsonValue] = {}
    return empty


def run(input_value: Input) -> JsonValue:
    probe = Probe(None)
    output: dict[str, JsonValue] = {}
    for name, case in input_value["cases"].items():
        converted: list[JsonValue] = []
        for value in case["records"]:
            try:
                model = probe.convert(record(value))
            except ValueError:
                converted.append(INVALID)
                continue
            converted.append(
                None if model is None else JSON.validate_json(model.model_dump_json())
            )
        output[name] = converted
    return output
