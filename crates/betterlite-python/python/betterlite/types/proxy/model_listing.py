from typing import Literal, NotRequired, TypedDict


class ModelInfoMetadata(TypedDict):
    fallbacks: list[str]


class ModelInfoResponse(TypedDict):
    id: str
    object: Literal["model"]
    created: int
    owned_by: str
    mode: NotRequired[str]
    max_input_tokens: NotRequired[int]
    max_output_tokens: NotRequired[int]
    metadata: NotRequired[ModelInfoMetadata]
