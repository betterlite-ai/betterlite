import litellm
from pydantic import JsonValue

from .inputs import Input
from .operations import (
    cohere_v3_input_type,
    titan_g1_request,
    titan_multimodal_image,
    titan_v2_dimensions_normalize,
    token_count_response,
)
from .transforms import TitanV2
from .views import response_out

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "titan_g1_request":
        return titan_g1_request(input_value)
    if input_value["function"] == "titan_v2_dimensions_normalize":
        return titan_v2_dimensions_normalize(input_value)
    if input_value["function"] == "titan_v2_binary_embedding":
        return response_out(
            TitanV2().replied(input_value["response_list"], input_value["model"])
        )
    if input_value["function"] == "titan_multimodal_image":
        return titan_multimodal_image(input_value)
    if input_value["function"] == "cohere_v3_input_type":
        return cohere_v3_input_type(input_value)
    return token_count_response(input_value)
