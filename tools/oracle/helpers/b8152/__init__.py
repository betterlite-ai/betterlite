from typing import Literal, TypeAlias, TypedDict

from litellm.llms.docker_model_runner.chat.transformation import DockerModelRunnerChatConfig
from litellm.llms.oobabooga.chat.transformation import OobaboogaConfig
from litellm.llms.xinference.image_generation.transformation import (
    XInferenceImageGenerationConfig as XinferenceConfig,
)
from pydantic import JsonValue

from b8152.oobabooga import (
    EmbeddingCall,
    ReplyCall,
    RequestCall,
    embedding_value,
    reply_value,
    request_value,
)
from b8152.servers import (
    CostCall,
    DockerCall,
    ParamsCall,
    ProviderCall,
    XinferenceCall,
    cost_value,
    docker_value,
    embed_value,
    params_value,
    provider_value,
    xinference_value,
)


class ProviderInfoInput(TypedDict):
    function: Literal["provider_info"]
    calls: list[ProviderCall]


class ChatParamsInput(TypedDict):
    function: Literal["chat_params"]
    calls: list[ParamsCall]


class DockerRequestInput(TypedDict):
    function: Literal["docker_request"]
    calls: list[DockerCall]


class LmStudioEmbedInput(TypedDict):
    function: Literal["lm_studio_embed"]
    optional_params: dict[str, JsonValue]


class XinferenceInput(TypedDict):
    function: Literal["xinference"]
    calls: list[XinferenceCall]


class CostInput(TypedDict):
    function: Literal["lemonade_cost"]
    calls: list[CostCall]


class OobaboogaRequestInput(TypedDict):
    function: Literal["oobabooga_request"]
    calls: list[RequestCall]


class OobaboogaEmbeddingInput(TypedDict):
    function: Literal["oobabooga_embedding"]
    calls: list[EmbeddingCall]


class OobaboogaReplyInput(TypedDict):
    function: Literal["oobabooga_reply"]
    calls: list[ReplyCall]


Input: TypeAlias = (
    ProviderInfoInput
    | ChatParamsInput
    | DockerRequestInput
    | LmStudioEmbedInput
    | XinferenceInput
    | CostInput
    | OobaboogaRequestInput
    | OobaboogaEmbeddingInput
    | OobaboogaReplyInput
)


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    if input_value["function"] == "provider_info":
        result = [provider_value(call) for call in input_value["calls"]]
    elif input_value["function"] == "chat_params":
        result = [params_value(call) for call in input_value["calls"]]
    elif input_value["function"] == "docker_request":
        docker = DockerModelRunnerChatConfig()
        result = [docker_value(docker, call) for call in input_value["calls"]]
    elif input_value["function"] == "lm_studio_embed":
        result = embed_value(input_value["optional_params"])
    elif input_value["function"] == "xinference":
        image = XinferenceConfig()
        result = [xinference_value(image, call) for call in input_value["calls"]]
    elif input_value["function"] == "lemonade_cost":
        result = [cost_value(call) for call in input_value["calls"]]
    elif input_value["function"] == "oobabooga_request":
        result = [request_value(call) for call in input_value["calls"]]
    elif input_value["function"] == "oobabooga_embedding":
        result = [embedding_value(call) for call in input_value["calls"]]
    else:
        oobabooga = OobaboogaConfig()
        result = [reply_value(oobabooga, call) for call in input_value["calls"]]
    return result
