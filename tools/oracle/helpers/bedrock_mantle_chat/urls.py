import litellm
from pydantic import JsonValue

from bedrock_invoke_primitives.stubs import sealed_environment

from .views import (
    CHAT,
    MESSAGES,
    OBJECT,
    EnvironmentArgs,
    Object,
    UrlCall,
    UrlCases,
)


def litellm_params_of(call: UrlCall) -> Object:
    params: Object = {}
    metadata = call["metadata"]
    if metadata:
        params["metadata"] = OBJECT.validate_python(metadata)
    project_id = call["project_id"]
    if project_id is not None:
        params["aws_bedrock_project_id"] = project_id
    return params


def environment_arguments(call: UrlCall, params: Object) -> EnvironmentArgs:
    return EnvironmentArgs(
        headers=dict(call["headers"]),
        model=call["model"],
        messages=[],
        optional_params=params,
        litellm_params=litellm_params_of(call),
        api_key=None,
        api_base=call["api_base"],
    )


def complete_url(call: UrlCall, params: Object) -> str:
    with sealed_environment(call["environment"]):
        return CHAT.get_complete_url(
            api_base=call["api_base"],
            api_key=None,
            model=call["model"],
            optional_params=params,
            litellm_params={},
            stream=None,
        )


def url_call(call: UrlCall, *, messages: bool) -> JsonValue:
    params = dict(call["optional_params"])
    arguments = environment_arguments(call, params)
    original = litellm.bedrock_request_metadata_fields
    fields = call["policy_fields"]
    litellm.bedrock_request_metadata_fields = [] if fields is None else list(fields)
    try:
        answer = message_environment(arguments) if messages else chat_environment(arguments)
    finally:
        litellm.bedrock_request_metadata_fields = original
    result: Object = {"url": complete_url(call, params), **answer}
    return result


def chat_environment(arguments: EnvironmentArgs) -> Object:
    merged = CHAT.validate_environment(**arguments)
    outcome: Object = {"headers": OBJECT.validate_python(merged)}
    return outcome


def message_environment(arguments: EnvironmentArgs) -> Object:
    merged, api_base = MESSAGES.validate_anthropic_messages_environment(**arguments)
    outcome: Object = {"headers": OBJECT.validate_python(merged), "api_base": api_base}
    return outcome


def urls(call: UrlCases) -> JsonValue:
    messages = call["function"] == "message_url"
    outcome: Object = {
        "calls": [url_call(entry, messages=messages) for entry in call["calls"]]
    }
    return outcome
