from typing import Protocol, TypeAlias, cast

from litellm.llms.voyage.embedding.transformation import VoyageEmbeddingConfig
from litellm.llms.voyage.embedding.transformation_contextual import (
    VoyageContextualEmbeddingConfig,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Input = str | list[str] | list[list[str]] | list[int] | list[list[int]]


class EmbeddingConfig(Protocol):
    def map_openai_params(
        self, non_default_params: dict[str, Json], optional_params: dict[str, Json],
        model: str, *, drop_params: bool
    ) -> dict[str, Json]: ...

    def transform_embedding_request(
        self,
        model: str,
        input_data: Input,
        optional_params: dict[str, Json],
        headers: dict[str, Json],
    ) -> dict[str, Json]: ...

    def get_complete_url(
        self, api_base: str | None, api_key: str | None, model: str,
        optional_params: dict[str, Json], litellm_params: dict[str, Json]
    ) -> str: ...


class ContextualConfig(EmbeddingConfig, Protocol):
    def is_contextualized_embeddings(self, model: str) -> bool: ...


def run(value: dict[str, Json]) -> Json:
    text = cast(dict[str, Json], value["text"])
    contextual = cast(dict[str, Json], value["contextual"])
    text_config = cast(EmbeddingConfig, VoyageEmbeddingConfig())
    contextual_config = cast(ContextualConfig, VoyageContextualEmbeddingConfig())
    text_input = cast(str | list[str], text["input"])
    contextual_input = cast(str | list[str] | list[list[str]], contextual["input"])
    text_options = cast(dict[str, Json], text["options"])
    contextual_options = cast(dict[str, Json], contextual["options"])
    text_model = cast(str, text["model"])
    contextual_model = cast(str, contextual["model"])
    text_base = cast(str | None, text["base"])
    contextual_base = cast(str | None, contextual["base"])
    text_params = text_config.map_openai_params(
        text_options, {}, text_model, drop_params=False
    )
    text_request = cast(
        dict[str, object],
        text_config.transform_embedding_request(
            text_model, text_input, text_params, {}
        ),
    )
    contextual_params = contextual_config.map_openai_params(
        contextual_options, {}, contextual_model, drop_params=False
    )
    contextual_request = cast(
        dict[str, object],
        contextual_config.transform_embedding_request(
            contextual_model, contextual_input, contextual_params, {}
        ),
    )
    return {
        "text": {
            "url": text_config.get_complete_url(text_base, None, text_model, text_options, {}),
            "request": cast(Json, text_request),
        },
        "contextual": {
            "url": contextual_config.get_complete_url(
                contextual_base, None, contextual_model, contextual_options, {}
            ),
            "request": cast(Json, contextual_request),
            "is_contextual": contextual_config.is_contextualized_embeddings(contextual_model),
        },
    }
