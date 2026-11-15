import httpx
from litellm.llms.databricks.embed.handler import DatabricksEmbeddingHandler
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from .common import (
    EmbeddingApi,
    EmbeddingCase,
    EmbeddingTransport,
    Input,
    Logging,
    UrlCase,
    guarded,
)
from .responses import responses_url_values


class EmbeddingHandler(DatabricksEmbeddingHandler):
    @override
    def __init__(self) -> None:
        pass


WATCHED = ["authorization", "content-type", "user-agent"]
TIMEOUT = 30.0


def embed(handler: EmbeddingApi, case: EmbeddingCase) -> dict[str, JsonValue]:
    transport = EmbeddingTransport()
    client = httpx.Client(transport=httpx.MockTransport(transport.handle))
    optional = case.get("optional_params")
    if optional is None:
        optional = {}
    asynchronous = False
    handler.embedding(
        case["model"],
        [case["input"]],
        TIMEOUT,
        Logging(),
        case.get("api_key"),
        case.get("api_base"),
        optional,
        None,
        client,
        asynchronous,
        case.get("custom_endpoint"),
        case.get("headers"),
    )
    return transport.output(WATCHED)


def endpoints(input_value: Input) -> JsonValue:
    embedding_cases = input_value.get("embedding")
    responses = input_value.get("responses")
    if embedding_cases is None or responses is None:
        raise ValueError("embedding and responses are required")
    embedded = TypeAdapter(list[EmbeddingCase]).validate_python(embedding_cases)
    urls = TypeAdapter(list[UrlCase]).validate_python(responses)
    handler = EmbeddingHandler()
    return {
        "embedding": [guarded(embed, handler, case) for case in embedded],
        "responses": responses_url_values(urls),
    }
