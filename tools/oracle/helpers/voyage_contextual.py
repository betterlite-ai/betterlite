import os
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.voyage.embedding.transformation import VoyageEmbeddingConfig
from litellm.llms.voyage.embedding.transformation_contextual import (
    VoyageContextualEmbeddingConfig,
)
from litellm.types.utils import EmbeddingResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Input = str | list[str] | list[list[str]] | list[int] | list[list[int]]


class EmbeddingConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, Json], model: str, messages: list[Json],
        optional_params: dict[str, Json], litellm_params: dict[str, Json],
        **credentials: str | None
    ) -> dict[str, Json]: ...

    def map_openai_params(
        self, non_default_params: dict[str, Json], optional_params: dict[str, Json],
        model: str, *, drop_params: bool
    ) -> dict[str, Json]: ...

    def transform_embedding_request(
        self, model: str, input_data: Input, optional_params: dict[str, Json],
        headers: dict[str, Json]
    ) -> dict[str, Json]: ...

    def get_complete_url(
        self, api_base: str | None, api_key: str | None, model: str,
        optional_params: dict[str, Json], litellm_params: dict[str, Json]
    ) -> str: ...

    def transform_embedding_response(
        self, model: str, raw_response: httpx.Response, model_response: EmbeddingResponse,
        logging_obj: object
    ) -> EmbeddingResponse: ...


def reply(config: EmbeddingConfig, model: str, value: Json) -> Json:
    if value is None:
        return None
    supplied = cast(dict[str, Json], value)
    raw = httpx.Response(cast(int, supplied["status"]), content=cast(str, supplied["body"]))
    response = config.transform_embedding_response(model, raw, EmbeddingResponse(), object())
    fields = cast(dict[str, Json], response.model_dump())
    usage = cast(dict[str, Json], fields["usage"])
    return {
        "model": fields["model"], "data": fields["data"], "object": fields["object"],
        "usage": {"prompt_tokens": usage["prompt_tokens"], "total_tokens": usage["total_tokens"]},
    }


def transform(config: EmbeddingConfig, value: dict[str, Json]) -> dict[str, Json]:
    options = cast(dict[str, Json], value["options"])
    model = cast(str, value["model"])
    params = {
        key: item for key, item in options.items() if key not in {"dimensions", "encoding_format"}
    }
    mapped = config.map_openai_params(options, params, model, drop_params=False)
    with patch.dict(os.environ, cast(dict[str, str], value.get("env", {})), clear=True):
        headers = config.validate_environment(
            dict(cast(dict[str, Json], value.get("headers", {}))), model, [], {}, {},
            api_key=cast(str | None, value.get("api_key")),
        )
    return {
        "headers": headers,
        "url": config.get_complete_url(cast(str | None, value["base"]), None, model, options, {}),
        "request": config.transform_embedding_request(
            model, cast(Input, value["input"]), mapped, {}
        ),
        "response": reply(config, model, value.get("reply")),
    }


def run(value: dict[str, Json]) -> Json:
    text = cast(dict[str, Json], value["text"])
    contextual = cast(dict[str, Json], value["contextual"])
    text_config = cast(EmbeddingConfig, VoyageEmbeddingConfig())
    contextual_config = cast(EmbeddingConfig, VoyageContextualEmbeddingConfig())
    text_result = transform(text_config, text)
    contextual_result = transform(contextual_config, contextual)
    contextual_result["is_contextual"] = (
        VoyageContextualEmbeddingConfig.is_contextualized_embeddings(
            cast(str, contextual["model"])
        )
    )
    return {"text": text_result, "contextual": contextual_result}
