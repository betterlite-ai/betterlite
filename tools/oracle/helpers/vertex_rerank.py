import functools
import uuid
from collections.abc import Callable
from datetime import UTC, datetime
from decimal import Decimal
from typing import Literal, NotRequired, Protocol, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.vertex_ai.rerank.transformation import VertexAIRerankConfig
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.rerank import RerankResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

JSON = TypeAdapter(JsonValue)
Params = dict[str, JsonValue]
Document = str | dict[str, JsonValue]
MapParams = Callable[
    [Params, str, bool, str, list[Document], str | None, int | None, list[str] | None, bool | None],
    Params,
]


class Config(Protocol):
    @property
    def map_cohere_rerank_params(self) -> MapParams: ...
    def get_complete_url(
        self, api_base: str | None, model: str, optional_params: Params | None
    ) -> str: ...
    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None, optional_params: Params
    ) -> dict[str, str]: ...
    def transform_rerank_request(
        self, model: str, optional_rerank_params: Params, headers: dict[str, str],
        litellm_params: Params | None,
    ) -> Params: ...
    def transform_rerank_response(
        self, model: str, raw_response: httpx.Response, model_response: RerankResponse,
        logging_obj: Logging, request_data: Params,
    ) -> RerankResponse: ...


class Reply(TypedDict):
    meta: dict[str, dict[str, int | None] | None] | None


REPLY = TypeAdapter(Reply)


class ScriptedRerank(VertexAIRerankConfig):
    def __init__(self, project: str) -> None:
        super().__init__()
        self.project = project
        self.lookups: list[JsonValue] = []

    @override
    def _ensure_access_token(
        self, credentials: VERTEX_CREDENTIALS_TYPES | None, project_id: str | None,
        custom_llm_provider: Literal["vertex_ai", "vertex_ai_beta", "gemini"],
    ) -> tuple[str, str]:
        self.lookups.append(
            {"credentials": JSON.validate_python(credentials), "project": project_id,
             "provider": custom_llm_provider}
        )
        return "token", self.project


class Input(TypedDict):
    model: str
    api_base: NotRequired[str | None]
    vertex_project: NotRequired[str | None]
    vertex_credentials: NotRequired[str | None]
    access_project: str
    headers: dict[str, str]
    query: str
    documents: list[str | dict[str, JsonValue]]
    top_n: NotRequired[int | None]
    return_documents: NotRequired[bool | None]
    non_default_params: dict[str, JsonValue]
    litellm_params: NotRequired[dict[str, JsonValue] | None]
    response: dict[str, JsonValue]


def run(data: Input) -> JsonValue:
    uuid.uuid4 = functools.partial(uuid.UUID, int=1)
    scripted = ScriptedRerank(data["access_project"])
    config: Config = cast(Config, scripted)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/rerank/transformation.py#L235
    optional: Params = {
        "vertex_project": data.get("vertex_project"),
        "vertex_credentials": data.get("vertex_credentials"),
    }
    url = config.get_complete_url(data.get("api_base"), data["model"], optional)
    headers = config.validate_environment(data["headers"], data["model"], None, optional)
    drop_params = False
    mapped = config.map_cohere_rerank_params(
        data["non_default_params"], data["model"], drop_params, data["query"], data["documents"],
        None, data.get("top_n"), None, data.get("return_documents"),
    )
    request = config.transform_rerank_request(
        data["model"], mapped, data["headers"], data.get("litellm_params")
    )
    answer = config.transform_rerank_response(
        model=data["model"],
        raw_response=httpx.Response(200, json=data["response"]),
        model_response=RerankResponse(),
        logging_obj=Logging(
            model=data["model"], messages=None, stream=False, call_type="rerank",
            start_time=datetime(2026, 1, 1, tzinfo=UTC), litellm_call_id="rerank-call",
            function_id="rerank-function",
        ),
        request_data=request,
    )
    ranked = answer.results
    meta = REPLY.validate_json(answer.model_dump_json())["meta"]
    if ranked is None or meta is None:
        raise ValueError("the rerank response has no results or meta")
    billed = meta.get("billed_units")
    return {
        "target": {
            "lookups": scripted.lookups,
            "url": url,
            "headers": JSON.validate_python(headers),
        },
        "rank": {
            "request": JSON.validate_python(request),
            "response": {
                "id": answer.id,
                "results": [
                    {"index": entry["index"],
                     "relevance_score": str(Decimal(repr(entry["relevance_score"])).normalize())}
                    for entry in ranked
                ],
                "search_units": None if billed is None else billed.get("search_units"),
            },
        },
    }
