from litellm.types.rerank import RerankResponse
from pydantic import JsonValue

import bedrock_other_rerank

from .common import (
    JSON,
    credentials_of,
    documents_of,
    params_of,
    query_value,
    reply_body,
    reply_headers,
    reply_status,
    request_row,
    success_row,
)
from .models import (
    RERANK_OUTPUT,
    RERANK_PROBE,
    RERANK_REQUEST,
    Probe,
)


def rerank_reply(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    failure: JsonValue = None
    if reply is not None and reply.get("failure") is True:
        failure = "failed"
    return {
        "status": reply_status(reply),
        "headers": reply_headers(reply),
        "body": reply_body(reply),
        "failure": failure,
    }


def rerank_documents(probe: Probe) -> dict[str, JsonValue]:
    raw: dict[str, JsonValue] = {"documents": JSON.validate_python(documents_of(probe))}
    raw["rank_fields"] = JSON.validate_python(probe.get("rank_fields"))
    raw["return_documents"] = probe.get("return_documents")
    raw["max_chunks_per_doc"] = None
    raw["optional_params"] = params_of(probe)
    return raw


def rerank_connection(probe: Probe) -> dict[str, JsonValue]:
    return {
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "api_base": probe.get("api_base"),
        "credentials": credentials_of(probe),
        "reply": rerank_reply(probe),
        "supplied": "sync",
        "is_async": False,
        "timeout_ms": probe.get("timeout_ms"),
    }


def rerank_probe(probe: Probe) -> dict[str, JsonValue]:
    raw: dict[str, JsonValue] = {"model": probe["model"], "query": query_value(probe)}
    raw["top_n"] = probe.get("top_n")
    raw.update(rerank_documents(probe))
    raw.update(rerank_connection(probe))
    return raw


def rerank_value(response: RerankResponse) -> JsonValue:
    results: JsonValue = None
    items = response.results
    if items is not None:
        entries: list[JsonValue] = [
            {"index": item["index"], "score": item["relevance_score"]} for item in items
        ]
        results = JSON.validate_python(entries)
    raw: dict[str, JsonValue] = {"id": response.id, "results": results}
    return JSON.validate_python(raw)


def rerank_row(probe: Probe) -> JsonValue:
    request = RERANK_REQUEST.validate_python(
        bedrock_other_rerank.prepare_value(RERANK_PROBE.validate_python(rerank_probe(probe)))
    )
    response = RERANK_OUTPUT.validate_python(
        bedrock_other_rerank.result_value(RERANK_PROBE.validate_python(rerank_probe(probe)))
    )
    authorization: str | None = None
    if request["signed_headers"]["authorization"]:
        authorization = "sigv4"
    requests: list[JsonValue] = [
        request_row(request["posted_url"], request["posted_body"], authorization)
    ]
    return success_row(requests, rerank_value(response))
