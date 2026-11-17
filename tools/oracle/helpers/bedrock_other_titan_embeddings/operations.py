from typing import Literal, NotRequired, TypedDict

from litellm.llms.bedrock.embed.amazon_titan_v2_transformation import AmazonTitanV2Config
from pydantic import JsonValue

from .probes import G1_CONFIG, V2_CONFIG
from .transforms import (
    ConfigProbe,
    TitanCohere,
    TitanG1,
    TitanMultimodal,
    TitanV2,
    mapped,
    params_report,
)
from .views import response_out


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


# each step constructs AmazonTitanV2Config(...), and the class keeps what an earlier step set
def v2_configs(settings: list[SettingsStep]) -> list[JsonValue]:
    configs: list[JsonValue] = []
    for step in settings:
        probe: ConfigProbe = AmazonTitanV2Config(**step)
        configs.append(probe.get_config())
    return configs


def titan_g1_request(call: G1Request) -> JsonValue:
    config = TitanG1()
    params = mapped(config, call["non_default"], call["optional_params"])
    out = params_report(config, call["non_default"], params)
    out["request"] = config.request(call["input"], call["inference_params"])
    out["inference_params"] = call["inference_params"]
    configs: dict[str, JsonValue] = {"g1": G1_CONFIG.get_config(), "v2": V2_CONFIG.get_config()}
    out["configs"] = configs
    return out


def titan_v2_dimensions_normalize(call: V2DimensionsNormalize) -> JsonValue:
    config = TitanV2()
    params = mapped(config, call["non_default"], call["optional_params"])
    out = params_report(config, call["non_default"], params)
    out["request"] = config.request(call["input"], call["inference_params"])
    out["configs"] = v2_configs(call["settings"])
    return out


def titan_multimodal_image(call: MultimodalImage) -> JsonValue:
    config = TitanMultimodal()
    params = mapped(config, call["non_default"], call["optional_params"])
    out = params_report(config, call["non_default"], params)
    out["request"] = config.request(call["input"], call["inference_params"])
    out["inference_params"] = call["inference_params"]
    return out


def cohere_v3_input_type(call: CohereV3InputType) -> JsonValue:
    config = TitanCohere()
    params = mapped(config, call["non_default"], call["optional_params"])
    out = params_report(config, call["non_default"], params)
    out["request"] = config.request(call["model"], call["input"], params)
    return out


def token_count_response(call: TokenCountResponse) -> JsonValue:
    if call["provider"] == "g1":
        return response_out(TitanG1().replied(call["response_list"], call["model"]))
    if call["provider"] == "v2":
        return response_out(TitanV2().replied(call["response_list"], call["model"]))
    response = TitanMultimodal().replied(
        call["response_list"], call["model"], call.get("batch_data")
    )
    return response_out(response)
