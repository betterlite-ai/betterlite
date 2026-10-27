import asyncio
import json
from typing import cast

from litellm.proxy.lens.analysis import examine_executions
from litellm.proxy.lens.models import (
    Claim,
    Coverage,
    ExecutionContent,
    ModelRequest,
    ModelResult,
    Sample,
)
from pydantic import BaseModel

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Page(BaseModel):
    cursor: str
    offset: int
    content: ExecutionContent


class Input(BaseModel):
    claim: Claim
    sample: Sample
    pages: tuple[Page, ...]
    responses: tuple[ModelResult, ...]


async def examine(data: Input) -> Json:
    coverage: list[Coverage] = []
    reads = 0
    models = 0
    active = 0
    maximum = 0

    async def read(execution: str, cursor: str, offset: int) -> ExecutionContent:
        nonlocal reads
        reads += 1
        return next(
            page.content
            for page in data.pages
            if page.content.execution.id == execution
            and page.cursor == cursor
            and page.offset == offset
        )

    async def model(_request: ModelRequest) -> ModelResult:
        nonlocal models, active, maximum
        response = data.responses[models]
        models += 1
        active += 1
        maximum = max(maximum, active)
        await asyncio.sleep(0.01)
        active -= 1
        return response

    async def progress(_stage: str, value: Coverage) -> None:
        coverage.append(value)

    examined = [
        item
        async for item in examine_executions(data.claim, data.sample, read, model, progress)
    ]
    examined.sort(key=lambda item: item.execution.id)
    result = [
        [json.loads(item.model_dump_json()) for item in examined],
        [json.loads(item.model_dump_json()) for item in coverage],
        reads,
        models,
        maximum <= data.claim.job.settings.concurrency,
    ]
    return cast("Json", result)


def run(value: dict[str, Json]) -> Json:
    data = Input.model_validate_json(json.dumps(value))
    return asyncio.run(examine(data))
