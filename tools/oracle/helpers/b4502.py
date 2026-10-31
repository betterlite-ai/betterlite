import asyncio
import json
from typing import Final, TypeAlias, TypeVar, cast

from litellm.proxy.lens.models import Evidence, Execution, LensSettings, MetadataFilter, Scope
from litellm.proxy.lens.sources import (
    SourceReader,
    execution_id,
    parameters,
    parse_execution,
    selection_id,
)
from pydantic import BaseModel

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Record = TypeVar("Record", bound=BaseModel)


def parse(model: type[Record], value: Json) -> Record:
    return model.model_validate_json(json.dumps(value))


def dump(record: BaseModel) -> Json:
    return cast("Json", json.loads(record.model_dump_json()))


class FakeStorage:
    def __init__(self, rows: list[Json]) -> None:
        self.rows: Final = rows

    async def lens_availability(self, parameters: object) -> list[Json]:  # noqa: ARG002
        return self.rows

    async def lens_agents(self, parameters: object) -> list[Json]:  # noqa: ARG002
        return self.rows

    async def lens_sample(self, parameters: object) -> list[Json]:  # noqa: ARG002
        return self.rows

    async def lens_content(self, parameters: object) -> list[Json]:  # noqa: ARG002
        return self.rows

    async def lens_evidence(self, parameters: object) -> list[Json]:  # noqa: ARG002
        return self.rows


async def availability(scope: Scope, rows: list[Json]) -> Json:
    return dump(await SourceReader(FakeStorage(rows)).availability(scope))


async def agents(scope: Scope, rows: list[Json]) -> Json:
    names = await SourceReader(FakeStorage(rows)).agents(scope)
    return list(names)


async def sample(
    scope: Scope, settings: LensSettings, window: dict[str, Json], rows: list[Json]
) -> Json:
    reader = SourceReader(FakeStorage(rows))
    result = await reader.sample(
        scope,
        settings,
        cast(int, window["start"]),
        cast(int, window["end"]),
        offset=cast(int, window.get("offset", 0)),
        page_size=cast(int, window["page_size"]),
        preview=cast(bool, window.get("preview", False)),
        cursor=cast(str, window.get("cursor", "")),
    )
    return dump(result)


async def content(
    scope: Scope, execution: Execution, cursor: str, offset: int, rows: list[Json]
) -> Json:
    reader = SourceReader(FakeStorage(rows))
    result = await reader.content(scope, execution, cursor, offset)
    return dump(result)


async def verify_evidence(
    scope: Scope, execution: Execution, evidence: Evidence, rows: list[Json]
) -> Json:
    reader = SourceReader(FakeStorage(rows))
    return await reader.verify_evidence(scope, execution, evidence)


def run(input_value: dict[str, Json]) -> Json:
    result: Json
    match input_value:
        case {
            "execution_id": {
                "source": str(source),
                "team_id": str(team_id),
                "trace_id": str(trace_id),
                "trace_ref": str(trace_ref),
            }
        }:
            result = execution_id(source, team_id, trace_id, trace_ref)
        case {"parse_execution": {"value": str(value)}}:
            result = list(parse_execution(value))
        case {"selection_id": {"value": str(value)}}:
            result = selection_id(value)
        case {"parameters": {"scope": scope, "filters": list(filters)}}:
            filter_models = tuple(parse(MetadataFilter, item) for item in filters)
            result = cast("Json", dict(parameters(parse(Scope, scope), filter_models)))
        case {"availability": {"scope": scope, "rows": list(rows)}}:
            result = asyncio.run(availability(parse(Scope, scope), rows))
        case {"agents": {"scope": scope, "rows": list(rows)}}:
            result = asyncio.run(agents(parse(Scope, scope), rows))
        case {
            "sample": {
                "scope": scope,
                "settings": settings,
                "window": dict(window),
                "rows": list(rows),
            }
        }:
            result = asyncio.run(
                sample(parse(Scope, scope), parse(LensSettings, settings), window, rows)
            )
        case {
            "content": {
                "scope": scope,
                "execution": execution,
                "cursor": str(cursor),
                "offset": int(offset),
                "rows": list(rows),
            }
        }:
            result = asyncio.run(
                content(parse(Scope, scope), parse(Execution, execution), cursor, offset, rows)
            )
        case {
            "verify_evidence": {
                "scope": scope,
                "execution": execution,
                "evidence": evidence,
                "rows": list(rows),
            }
        }:
            result = asyncio.run(
                verify_evidence(
                    parse(Scope, scope),
                    parse(Execution, execution),
                    parse(Evidence, evidence),
                    rows,
                )
            )
        case _:
            raise ValueError("b4502 input names no known operation")
    return result
