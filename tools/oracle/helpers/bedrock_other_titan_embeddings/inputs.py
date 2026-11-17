from typing import Literal, NotRequired, TypedDict

from pydantic import JsonValue


class SettingsStep(TypedDict, total=False):
    normalize: bool | None
    dimensions: int | None


class G1Request(TypedDict):
    function: Literal["titan_g1_request"]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    input: str
    inference_params: dict[str, JsonValue]


class V2DimensionsNormalize(TypedDict):
    function: Literal["titan_v2_dimensions_normalize"]
    settings: list[SettingsStep]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    input: str
    inference_params: dict[str, JsonValue]


class V2BinaryEmbedding(TypedDict):
    function: Literal["titan_v2_binary_embedding"]
    model: str
    response_list: list[dict[str, JsonValue]]


class MultimodalImage(TypedDict):
    function: Literal["titan_multimodal_image"]
    non_default: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    input: str
    inference_params: dict[str, JsonValue]


class CohereV3InputType(TypedDict):
    function: Literal["cohere_v3_input_type"]
    model: str
    input: list[str]
    optional_params: dict[str, JsonValue]
    non_default: dict[str, JsonValue]


class TokenCountResponse(TypedDict):
    function: Literal["token_count_response"]
    provider: Literal["g1", "v2", "multimodal"]
    model: str
    response_list: list[dict[str, JsonValue]]
    batch_data: NotRequired[list[dict[str, JsonValue]] | None]


Input = (
    G1Request
    | V2DimensionsNormalize
    | V2BinaryEmbedding
    | MultimodalImage
    | CohereV3InputType
    | TokenCountResponse
)
