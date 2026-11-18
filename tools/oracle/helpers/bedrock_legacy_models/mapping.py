import logging
from collections.abc import Generator
from contextlib import contextmanager
from typing import TypedDict

import litellm
from pydantic import JsonValue as Json
from pydantic import TypeAdapter
from typing_extensions import override

from .constructors import FamilyName, LegacyClass, construct


class Warnings(logging.Handler):
    def __init__(self) -> None:
        super().__init__(level=logging.WARNING)
        self.messages: list[str] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        self.messages.append(record.getMessage())

    def first(self) -> str | None:
        return self.messages[0] if self.messages else None


class MapCase(TypedDict):
    family: FamilyName
    non_default: dict[str, Json]
    optional: dict[str, Json]
    model: str
    drop_params: bool
    global_drop_params: bool


class Mappings(TypedDict):
    cases: list[MapCase]


@contextmanager
def capturing(*, global_drop_params: bool) -> Generator[Warnings]:
    warnings = Warnings()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(warnings)
    previous = litellm.drop_params
    litellm.drop_params = global_drop_params
    try:
        yield warnings
    finally:
        litellm.drop_params = previous
        logger.removeHandler(warnings)


def mapping(case: MapCase) -> Json:
    with capturing(global_drop_params=case["global_drop_params"]) as warnings:
        mapper: LegacyClass = construct(case["family"], {})
        mapped = mapper.map_openai_params(
            non_default_params=case["non_default"],
            optional_params=dict(case["optional"]),
            model=case["model"],
            drop_params=case["drop_params"],
        )
        supported: list[Json] = [*mapper.get_supported_openai_params(case["model"])]
    return {"supported": supported, "mapped": mapped, "warning": warnings.first()}


def mappings(raw: Json) -> Json:
    cases = TypeAdapter(Mappings).validate_python(raw)
    results: list[Json] = [mapping(case) for case in cases["cases"]]
    return {"results": results}
