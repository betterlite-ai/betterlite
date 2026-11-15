from litellm.llms.openai.containers.transformation import OpenAIContainerConfig
from pydantic import JsonValue

from .cases import Input
from .contracts import JSON, ContainerConfig
from .sections import (
    content,
    content_reply,
    create,
    create_reply,
    error,
    file_list_request,
    headers,
    join_path,
    list_request,
    map_params,
    metadata_reply,
    resource,
    url,
)


def run(input_value: Input) -> JsonValue:
    config: ContainerConfig = OpenAIContainerConfig()
    answer: dict[str, JsonValue] = {}
    params = config.get_supported_openai_params() if input_value["supported"] else []
    answer["supported"] = JSON.validate_python(params)
    answer["maps"] = [map_params(config, call) for call in input_value["maps"]]
    answer["joins"] = [join_path(call) for call in input_value["joins"]]
    answer["urls"] = [url(config, call) for call in input_value["urls"]]
    answer["creates"] = [create(config, call) for call in input_value["creates"]]
    answer["lists"] = [list_request(config, call) for call in input_value["lists"]]
    answer["file_lists"] = [
        file_list_request(config, call) for call in input_value["file_lists"]
    ]
    answer["resources"] = [resource(config, call) for call in input_value["resources"]]
    answer["contents"] = [content(config, call) for call in input_value["contents"]]
    answer["headers"] = [headers(config, call) for call in input_value["headers"]]
    answer["create_replies"] = [
        create_reply(config, call) for call in input_value["create_replies"]
    ]
    answer["metadata_replies"] = [
        metadata_reply(config, call) for call in input_value["metadata_replies"]
    ]
    answer["content_replies"] = [
        content_reply(config, call) for call in input_value["content_replies"]
    ]
    answer["errors"] = [error(config, call) for call in input_value["errors"]]
    return JSON.validate_python(answer)
