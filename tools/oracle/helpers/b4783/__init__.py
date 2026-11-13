from __future__ import annotations

import asyncio
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from fastapi import HTTPException
from litellm.proxy._types import UserAPIKeyAuth
from litellm.proxy.agent_endpoints.agent_search import (
    AgentSearchEmbeddingFailed,
    AgentSearchHit,
    AgentSearchHits,
    AgentSearchIndex,
    AgentSearchNotConfigured,
    agent_search_result,
    agent_search_text,
    search_agents,
)
from litellm.proxy.utils import ProxyLogging
from litellm.types.agents import AgentResponse

if TYPE_CHECKING:
    from collections.abc import Awaitable, Callable, Sequence

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Vector: TypeAlias = tuple[float, ...]
Vectors: TypeAlias = dict[str, list[float]]
Outcome: TypeAlias = AgentSearchHits | AgentSearchNotConfigured | AgentSearchEmbeddingFailed


class VectorEmbedder:
    def __init__(self, vectors: Vectors, failure: dict[str, Json] | None) -> None:
        self.vectors = vectors
        self.failure = failure

    def __call__(self, texts: Sequence[str]) -> Awaitable[Sequence[Vector]]:
        return self.embed(texts)

    async def embed(self, texts: Sequence[str]) -> Sequence[Vector]:
        failure = self.failure
        if failure is not None:
            if cast(str, failure["kind"]) == "http":
                raise HTTPException(
                    status_code=cast(int, failure["status_code"]),
                    detail=cast(str, failure["detail"]),
                )
            raise ValueError(cast(str, failure["message"]))
        return [tuple(self.vectors[text]) for text in texts]


def agent_from(value: Json) -> AgentResponse:
    return AgentResponse.model_validate(value)


def agents_from(value: Json) -> list[AgentResponse]:
    return [agent_from(item) for item in cast("list[Json]", value)]


def outcome_json(outcome: Outcome) -> Json:
    if isinstance(outcome, AgentSearchHits):
        hits: Json = [
            {"agent_id": hit.agent.agent_id, "score": hit.score} for hit in outcome.hits
        ]
        return {"hits": hits}
    if isinstance(outcome, AgentSearchNotConfigured):
        return {"not_configured": {"reason": outcome.reason}}
    return {"failed": {"reason": outcome.reason}}


def run(input_value: dict[str, Json]) -> Json:
    function = cast(str, input_value["function"])
    handlers: dict[str, Callable[[dict[str, Json]], Json]] = {
        "search_text": search_text,
        "result": project,
        "rank": rank,
        "search_agents": search,
        "failures": failures,
    }
    handler = handlers.get(function)
    if handler is None:
        raise ValueError(f"unknown {litellm.__name__} helper function {function}")
    return handler(input_value)


def search_text(input_value: dict[str, Json]) -> Json:
    return agent_search_text(agent_from(input_value["agent"]))


def project(input_value: dict[str, Json]) -> Json:
    hit = AgentSearchHit(
        agent=agent_from(input_value["agent"]),
        score=cast(float, input_value["score"]),
    )
    return cast(Json, agent_search_result(hit).model_dump())


def rank(input_value: dict[str, Json]) -> Json:
    vectors = cast(Vectors, input_value["vectors"])
    failure = cast("dict[str, Json] | None", input_value.get("failure"))
    outcome = asyncio.run(
        AgentSearchIndex().search(
            cast(str, input_value["query"]),
            agents_from(input_value["agents"]),
            cast(int, input_value["top_k"]),
            VectorEmbedder(vectors, failure),
            cast(str, input_value["embedding_model"]),
        )
    )
    return outcome_json(outcome)


def search(input_value: dict[str, Json]) -> Json:
    index = AgentSearchIndex()
    query = cast(str, input_value["query"])
    agents = agents_from(input_value["agents"])
    top_k = cast(int, input_value["top_k"])
    model = cast(str, input_value["embedding_model"])
    without_model = asyncio.run(
        search_agents(
            query=query,
            agents=agents,
            top_k=top_k,
            router=None,
            embedding_model=None,
            index=index,
            user_api_key_dict=cast(UserAPIKeyAuth, None),
            proxy_logging_obj=cast(ProxyLogging, None),
        )
    )
    without_router = asyncio.run(
        search_agents(
            query=query,
            agents=agents,
            top_k=top_k,
            router=None,
            embedding_model=model,
            index=index,
            user_api_key_dict=cast(UserAPIKeyAuth, None),
            proxy_logging_obj=cast(ProxyLogging, None),
        )
    )
    return {
        "model_missing": outcome_json(without_model),
        "router_missing": outcome_json(without_router),
    }


def failures(input_value: dict[str, Json]) -> Json:
    index = AgentSearchIndex()
    query = cast(str, input_value["query"])
    agents = agents_from(input_value["agents"])
    top_k = cast(int, input_value["top_k"])
    model = cast(str, input_value["embedding_model"])
    failed = asyncio.run(
        index.search(
            query,
            agents,
            top_k,
            VectorEmbedder({}, {"kind": "value", "message": cast(str, input_value["message"])}),
            model,
        )
    )
    rejected = VectorEmbedder(
        {},
        {
            "kind": "http",
            "status_code": cast(int, input_value["http_status"]),
            "detail": cast(str, input_value["http_detail"]),
        },
    )
    http_error: Json = None
    try:
        asyncio.run(index.search(query, agents, top_k, rejected, model))
    except HTTPException as error:
        http_error = {"status_code": error.status_code, "detail": error.detail}
    return {"embedding_failed": outcome_json(failed), "http_error": http_error}
