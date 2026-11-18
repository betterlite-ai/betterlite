from datetime import UTC, datetime
from typing import TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.bedrock.search.transformation import AgentCoreSearchConfig
from pydantic import TypeAdapter


class Input(TypedDict):
    status: int
    headers: list[tuple[str, str]]
    body: str


class ResultView(TypedDict):
    title: str
    url: str
    snippet: str
    date: str | None
    last_updated: str | None


class ReplyView(TypedDict):
    results: list[ResultView]
    object: str


REPLY: TypeAdapter[ReplyView] = TypeAdapter(ReplyView)


def run(input_value: Input) -> ReplyView:
    response = httpx.Response(
        input_value["status"],
        content=input_value["body"].encode(),
        headers=input_value["headers"],
    )
    logging_obj = Logging(
        model="agentcore-search",
        messages=None,
        stream=False,
        call_type="search",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="agentcore-search-call",
        function_id="agentcore-search-function",
    )
    answer = AgentCoreSearchConfig().transform_search_response(response, logging_obj)
    return REPLY.validate_json(answer.model_dump_json())
