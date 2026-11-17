from litellm.llms.bedrock.embed.amazon_titan_v2_transformation import AmazonTitanV2Config
from pydantic import JsonValue

from .inputs import (
    CohereV3InputType,
    G1Request,
    MultimodalImage,
    SettingsStep,
    TokenCountResponse,
    V2DimensionsNormalize,
)
from .probes import G1_CONFIG, V2_CONFIG
from .support import ConfigProbe
from .transforms import TitanCohere, TitanG1, TitanMultimodal, TitanV2, mapped, params_report
from .views import response_out


# each step is one AmazonTitanV2Config(...) construction, and the class keeps what an earlier one set
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
