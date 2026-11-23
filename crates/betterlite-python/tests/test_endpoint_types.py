import importlib
from typing import ClassVar, Final, Protocol

import pytest
from pydantic import BaseModel, JsonValue, ValidationError
from typing_extensions import get_type_hints

from betterlite.types import embedding as betterlite_embedding
from betterlite.types import rerank as betterlite_rerank
from betterlite.types.images import main as betterlite_images
from betterlite.types.utils.base import ResponseObject

DEFAULT_TIMEOUT: Final = 600
TRUNCATE_TOKENS: Final = 4
OTHER_TOKENS: Final = 9
FILLED_INPUT: Final[list[str]] = ["a", "b"]
FILLED_TIMEOUT: Final = 5
EXTRA_DIMENSIONS: Final = 8
RESULT_INDEX: Final = 0
RELEVANCE: Final = 0.9


class TypedDictClass(Protocol):
    __required_keys__: ClassVar[frozenset[str]]
    __optional_keys__: ClassVar[frozenset[str]]
    __readonly_keys__: ClassVar[frozenset[str]]
    __total__: ClassVar[bool]


class ModelClasses(Protocol):
    def __getattr__(self, name: str) -> type[BaseModel]: ...


class TypedDicts(Protocol):
    def __getattr__(self, name: str) -> TypedDictClass: ...


THEIR_EMBEDDING: ModelClasses = importlib.import_module("litellm.types.embedding")
THEIR_RERANK: ModelClasses = importlib.import_module("litellm.types.rerank")
THEIR_RERANK_TYPES: TypedDicts = importlib.import_module("litellm.types.rerank")
THEIR_IMAGES: TypedDicts = importlib.import_module("litellm.types.images.main")

TYPED_DICTS: dict[str, tuple[TypedDictClass, TypedDictClass]] = {
    "OptionalRerankParams": (
        betterlite_rerank.OptionalRerankParams,
        THEIR_RERANK_TYPES.OptionalRerankParams,
    ),
    "RerankBilledUnits": (
        betterlite_rerank.RerankBilledUnits,
        THEIR_RERANK_TYPES.RerankBilledUnits,
    ),
    "RerankTokens": (betterlite_rerank.RerankTokens, THEIR_RERANK_TYPES.RerankTokens),
    "RerankResponseMeta": (
        betterlite_rerank.RerankResponseMeta,
        THEIR_RERANK_TYPES.RerankResponseMeta,
    ),
    "RerankResponseDocument": (
        betterlite_rerank.RerankResponseDocument,
        THEIR_RERANK_TYPES.RerankResponseDocument,
    ),
    "RerankResponseResult": (
        betterlite_rerank.RerankResponseResult,
        THEIR_RERANK_TYPES.RerankResponseResult,
    ),
    "ImageEditOptionalRequestParams": (
        betterlite_images.ImageEditOptionalRequestParams,
        THEIR_IMAGES.ImageEditOptionalRequestParams,
    ),
    "ImageEditRequestParams": (
        betterlite_images.ImageEditRequestParams,
        THEIR_IMAGES.ImageEditRequestParams,
    ),
}

RESULTS: Final[list[dict[str, JsonValue]]] = [
    {"index": RESULT_INDEX, "relevance_score": RELEVANCE}
]


def required_fields(model: type[BaseModel]) -> list[bool]:
    fields = model.model_fields
    return [field.is_required() for field in fields.values()]


def compare_dump(ours: ResponseObject, theirs: BaseModel) -> dict[str, JsonValue]:
    dumped: dict[str, JsonValue] = ours.model_dump()
    reference: dict[str, JsonValue] = theirs.model_dump()
    assert dumped == reference
    return dumped


def test_embedding_request_fields() -> None:
    ours = betterlite_embedding.EmbeddingRequest.model_fields
    theirs = THEIR_EMBEDDING.EmbeddingRequest.model_fields
    assert list(ours) == list(theirs)
    assert required_fields(betterlite_embedding.EmbeddingRequest) == required_fields(
        THEIR_EMBEDDING.EmbeddingRequest
    )


def test_embedding_request_dump() -> None:
    dumped = compare_dump(
        betterlite_embedding.EmbeddingRequest(model="m"),
        THEIR_EMBEDDING.EmbeddingRequest(model="m"),
    )
    assert dumped["input"] == []
    assert dumped["timeout"] == DEFAULT_TIMEOUT


def test_embedding_request_filled() -> None:
    dumped = compare_dump(
        betterlite_embedding.EmbeddingRequest(
            model="m", input=FILLED_INPUT, timeout=FILLED_TIMEOUT
        ),
        THEIR_EMBEDDING.EmbeddingRequest(model="m", input=FILLED_INPUT, timeout=FILLED_TIMEOUT),
    )
    assert dumped["timeout"] == FILLED_TIMEOUT


def test_embedding_request_extra() -> None:
    dumped = compare_dump(
        betterlite_embedding.EmbeddingRequest(model="m", dimensions=EXTRA_DIMENSIONS),
        THEIR_EMBEDDING.EmbeddingRequest(model="m", dimensions=EXTRA_DIMENSIONS),
    )
    assert dumped["dimensions"] == EXTRA_DIMENSIONS


def test_rerank_request_fields() -> None:
    ours = betterlite_rerank.RerankRequest.model_fields
    theirs = THEIR_RERANK.RerankRequest.model_fields
    assert list(ours) == list(theirs)
    assert required_fields(betterlite_rerank.RerankRequest) == required_fields(
        THEIR_RERANK.RerankRequest
    )


def test_rerank_request_dump() -> None:
    documents: list[str | dict[str, JsonValue]] = ["a", {"text": "b"}]
    dumped = compare_dump(
        betterlite_rerank.RerankRequest(model="m", query="q", documents=documents),
        THEIR_RERANK.RerankRequest(model="m", query="q", documents=documents),
    )
    assert dumped["top_n"] is None
    assert dumped["return_documents"] is None


def test_rerank_request_extra() -> None:
    dumped = compare_dump(
        betterlite_rerank.RerankRequest(model="m", query="q", documents=[], bogus=1),
        THEIR_RERANK.RerankRequest(model="m", query="q", documents=[], bogus=1),
    )
    assert "bogus" not in dumped


def test_truncation_params_fields() -> None:
    ours = betterlite_rerank.HostedVLLMRerankTruncationParams.model_fields
    theirs = THEIR_RERANK.HostedVLLMRerankTruncationParams.model_fields
    assert list(ours) == list(theirs)


def test_truncation_params_frozen() -> None:
    model = betterlite_rerank.HostedVLLMRerankTruncationParams(
        truncate_prompt_tokens=TRUNCATE_TOKENS
    )
    reference = THEIR_RERANK.HostedVLLMRerankTruncationParams(
        truncate_prompt_tokens=TRUNCATE_TOKENS
    )
    assert betterlite_rerank.HostedVLLMRerankTruncationParams.model_config.get("frozen") is True
    assert THEIR_RERANK.HostedVLLMRerankTruncationParams.model_config.get("frozen") is True
    with pytest.raises(ValidationError):
        model.truncate_prompt_tokens = OTHER_TOKENS
    assert model.truncate_prompt_tokens == TRUNCATE_TOKENS
    dumped = compare_dump(model, reference)
    assert dumped["truncate_prompt_tokens"] == TRUNCATE_TOKENS


def test_rerank_response_fields() -> None:
    ours = betterlite_rerank.RerankResponse.model_fields
    theirs = THEIR_RERANK.RerankResponse.model_fields
    assert list(ours) == list(theirs)


def test_rerank_response_dump() -> None:
    dumped = compare_dump(
        betterlite_rerank.RerankResponse(id="r", results=RESULTS, bogus=1),
        THEIR_RERANK.RerankResponse(id="r", results=RESULTS, bogus=1),
    )
    assert dumped["meta"] is None
    assert "bogus" not in dumped


def test_rerank_response_access() -> None:
    response = betterlite_rerank.RerankResponse(
        id="r", results=[{"index": RESULT_INDEX, "relevance_score": RELEVANCE}]
    )
    assert response["id"] == "r"
    assert response.get("id") == "r"
    assert response.get("missing") is None
    assert "id" in response
    assert "missing" not in response


def test_rerank_missing_key() -> None:
    response = betterlite_rerank.RerankResponse(id="r")
    with pytest.raises(AttributeError):
        _ = response["missing"]
    with pytest.raises(AttributeError):
        _ = response["_hidden_params"]
    assert response.get("_hidden_params") is None
    assert "_hidden_params" not in response


def test_rerank_hidden_params() -> None:
    first = betterlite_rerank.RerankResponse(id="a")
    second = betterlite_rerank.RerankResponse(id="b")
    reference = THEIR_RERANK.RerankResponse(id="a")
    first_private: dict[str, JsonValue] | None = first.__pydantic_private__
    second_private: dict[str, JsonValue] | None = second.__pydantic_private__
    reference_private: dict[str, JsonValue] | None = reference.__pydantic_private__
    assert first_private is not None
    assert second_private is not None
    assert reference_private is not None
    assert first_private["_hidden_params"] == reference_private["_hidden_params"]
    assert first_private["_hidden_params"] == {}
    assert first_private["_hidden_params"] is not second_private["_hidden_params"]
    assert "_hidden_params" not in first.model_dump()


@pytest.mark.parametrize("name", list(TYPED_DICTS))
def test_typed_dict_shape(name: str) -> None:
    ours, theirs = TYPED_DICTS[name]
    assert ours.__required_keys__ == theirs.__required_keys__, name
    assert ours.__optional_keys__ == theirs.__optional_keys__, name
    assert ours.__readonly_keys__ == theirs.__readonly_keys__, name
    assert ours.__total__ == theirs.__total__, name
    our_hints: dict[str, type] = get_type_hints(ours)
    their_hints: dict[str, type] = get_type_hints(theirs)
    assert list(our_hints) == list(their_hints), name
