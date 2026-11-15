from typing import Literal, Required, TypeAlias

from typing_extensions import TypedDict


class ImageEmbeddingInput(TypedDict, total=False):
    image: Required[str]
    text: str


EncodingFormat: TypeAlias = Literal["base64", "binary", "float", "int8", "ubinary", "uint8"]


class ImageEmbeddingRequest(TypedDict, total=False):
    input: Required[list[ImageEmbeddingInput]]
    dimensions: int
    encoding_format: EncodingFormat
