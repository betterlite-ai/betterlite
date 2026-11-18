import litellm
from pydantic import JsonValue

from .credentials import embedding_credentials
from .http import http_error, provider_json_response
from .signing import signed_embedding_request
from .transforms import transform_case
from .types import Input

litellm.suppress_debug_info = True


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "signed_embedding_request":
        return signed_embedding_request(input_value)
    if input_value["function"] == "provider_json_response":
        return provider_json_response(input_value)
    if input_value["function"] == "http_error":
        return http_error(input_value)
    if input_value["function"] == "embedding_credentials":
        return embedding_credentials(input_value)
    if input_value["function"] == "embedding_usage":
        return transform_case(input_value)
    if input_value["function"] == "async_embedding_single_arn":
        return transform_case(input_value)
    raise ValueError("the input names no bedrock embedding function")
