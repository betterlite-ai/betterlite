from typing import TypeAlias, TypedDict, cast

from litellm.exceptions import ContentPolicyViolationError
from litellm.llms.azure.exception_mapping import AzureOpenAIExceptionMapping

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    message: str
    model: str
    extra_information: str
    body: Json
    response: Json


class OriginalError(Exception):
    body: Json
    response: Json


def run(argument: Input) -> object:
    original = OriginalError()
    original.body = argument["body"]
    original.response = argument["response"]
    try:
        AzureOpenAIExceptionMapping.create_content_policy_violation_error(
            message=argument["message"],
            model=argument["model"],
            extra_information=argument["extra_information"],
            original_exception=original,
        )
    except ContentPolicyViolationError as error:
        body = cast("dict[str, Json]", error.body)
        provider_fields = cast("dict[str, Json]", error.provider_specific_fields)
        return cast("dict[str, Json]", {
            "class": "ContentPolicyViolation",
            "status": error.status_code,
            "message": error.message,
            "type": body["type"],
            "code": body["code"],
            "param": body["param"],
            "model": error.model,
            "provider": error.llm_provider,
            "litellm_debug_info": error.litellm_debug_info,
            "response": argument["response"],
            "body": body,
            "provider_specific_fields": provider_fields,
        })
    raise AssertionError("Azure content policy mapping returned")
