import asyncio
import os
from typing import Final, Literal, TypedDict

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import get_async_httpx_client
from litellm.proxy.public_endpoints.public_endpoints import (
    get_complexity_scorer_defaults,
    get_public_autorouter_presets,
    get_public_fuse_presets,
)
from litellm.router_strategy.complexity_router.fuse_presets import FusePresetCatalog
from litellm.types.llms.custom_http import httpxSpecialProvider
from litellm.types.proxy.public_endpoints.public_endpoints import (
    AutoRouterPresetRecord,
    ComplexityScorerDefaults,
)
from pydantic import JsonValue, TypeAdapter

LOCAL_ONLY_ENV: Final = "LITELLM_LOCAL_AUTOROUTER_PRESETS"


class RemoteResponse(TypedDict):
    status: int
    body: str


class AutorouterCall(TypedDict):
    function: Literal["autorouter_presets"]
    url: str
    local_only: bool
    responses: list[RemoteResponse]
    resolutions: int


class ScorerCall(TypedDict):
    function: Literal["scorer_defaults"]


class FuseCall(TypedDict):
    function: Literal["fuse_presets"]


Call = AutorouterCall | ScorerCall | FuseCall


class AutorouterAnswer(TypedDict):
    presets: JsonValue
    requests: int
    urls: list[str]


JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
RECORDS: Final[TypeAdapter[dict[str, AutoRouterPresetRecord]]] = TypeAdapter(
    dict[str, AutoRouterPresetRecord]
)
SCORER: Final[TypeAdapter[ComplexityScorerDefaults]] = TypeAdapter(
    ComplexityScorerDefaults
)
FUSE: Final[TypeAdapter[FusePresetCatalog]] = TypeAdapter(FusePresetCatalog)
ANSWER: Final[TypeAdapter[AutorouterAnswer]] = TypeAdapter(AutorouterAnswer)


class PresetResponder:
    def __init__(self, responses: list[RemoteResponse]) -> None:
        self.responses = responses
        self.requests: list[str] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.requests.append(str(request.url))
        index = min(len(self.requests) - 1, len(self.responses) - 1)
        response = self.responses[index]
        return httpx.Response(
            response["status"], text=response["body"], request=request
        )


async def autorouter_answer(call: AutorouterCall) -> JsonValue:
    litellm.autorouter_presets_url = call["url"]
    if call["local_only"]:
        os.environ[LOCAL_ONLY_ENV] = "true"
    else:
        os.environ.pop(LOCAL_ONLY_ENV, None)
    handler = get_async_httpx_client(llm_provider=httpxSpecialProvider.UI)
    responder = PresetResponder(call["responses"])
    client = httpx.AsyncClient(transport=httpx.MockTransport(responder))
    handler.client = client
    try:
        presets = await get_public_autorouter_presets()
        for _ in range(call["resolutions"] - 1):
            presets = await get_public_autorouter_presets()
    finally:
        await client.aclose()
    records = RECORDS.validate_python(presets)
    answer: AutorouterAnswer = {
        "presets": JSON.validate_json(RECORDS.dump_json(records)),
        "requests": len(responder.requests),
        "urls": responder.requests,
    }
    return JSON.validate_json(ANSWER.dump_json(answer))


async def scorer_answer() -> JsonValue:
    defaults = await get_complexity_scorer_defaults()
    return JSON.validate_json(SCORER.dump_json(defaults))


async def fuse_answer() -> JsonValue:
    catalog = await get_public_fuse_presets()
    return JSON.validate_json(FUSE.dump_json(catalog))


def run(input_value: Call) -> JsonValue:
    match input_value["function"]:
        case "autorouter_presets":
            return asyncio.run(autorouter_answer(input_value))
        case "scorer_defaults":
            return asyncio.run(scorer_answer())
        case "fuse_presets":
            return asyncio.run(fuse_answer())
