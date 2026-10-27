from typing import TypeAlias, cast

from litellm.repositories.base_repository import BaseRepository, DbRecord, record_to_dict
from litellm.repositories.prisma_protocols import TableActions
from pydantic import BaseModel, ValidationError

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class DictRecord(dict[str, object]):
    def dict(self) -> dict[str, object]:
        return cast(dict[str, object], self["dict"])


class DumpRecord(DictRecord):
    def model_dump(self) -> dict[str, object]:
        return cast(dict[str, object], self["model_dump"])


def record(value: Json) -> DbRecord | None:
    if value is None:
        return None
    fields = cast(dict[str, Json], value)
    if "model_dump" in fields:
        return DumpRecord(fields)
    if "dict" in fields:
        return DictRecord(fields)
    if "pairs" in fields:
        return cast(list[tuple[str, object]], fields["pairs"])
    mapping = cast(dict[str, object], fields.get("mapping", {}))
    if record_to_dict(mapping) is not mapping:
        raise ValueError("mapping identity changed")
    return mapping


class Model(BaseModel):
    id: int


class Repository(BaseRepository[Model]):
    @property
    def table(self) -> TableActions[DbRecord]:
        raise RuntimeError("pure helper has no table")

    models = BaseRepository[Model]._to_model_list  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]

    @property
    def model_class(self) -> type[Model]:
        return Model

    def _to_model(self, record: DbRecord | None) -> Model | None:
        model = super()._to_model(record)
        return None if model is not None and model.id == -1 else model

    model = _to_model


def run(input_value: dict[str, Json]) -> Json:
    repository = Repository(None)
    result: dict[str, Json] = {}
    for name, case in cast(dict[str, dict[str, Json]], input_value["cases"]).items():
        records = [record(value) for value in cast(list[Json], case["records"])]
        try:
            if case["operation"] == "projection":
                output = [None if value is None else record_to_dict(value) for value in records]
            elif case["operation"] == "list":
                output = [model.model_dump() for model in repository.models(records)]
            else:
                models = [repository.model(value) for value in records]
                output = [None if model is None else model.model_dump() for model in models]
            result[name] = cast(Json, output)
        except ValidationError:
            result[name] = None
    return result
