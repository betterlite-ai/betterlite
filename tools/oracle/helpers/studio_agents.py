import logging
import os
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.gemini.agents.transformation import GeminiAgentsConfig
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

Headers: TypeAlias = dict[str, str]


class Request(TypedDict):
    function: Literal["validate", "create_body", "list_request", "versions_request"]
    name: str
    headers: NotRequired[Headers]
    environment: NotRequired[Headers]
    api_key: NotRequired[str]
    api_base: NotRequired[str]
    base_agent: NotRequired[str]
    instructions: NotRequired[str]
    base_environment: NotRequired[str | dict[str, JsonValue]]
    page_size: NotRequired[int]
    page_token: NotRequired[str]


class Response(TypedDict):
    name: str
    status: int
    body: str
    headers: NotRequired[Headers]


class Reply(Response):
    function: Literal["reply"]
    operation: Literal["create", "get", "list", "versions"]


class Delete(Response):
    function: Literal["delete"]


class Input(TypedDict):
    calls: list[Request | Reply | Delete]


INPUT: Final = TypeAdapter(Input)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])

API: Final = GeminiAgentsConfig()


class WarningRecorder(logging.Handler):
    def __init__(self) -> None:
        super().__init__()
        self.subjects: list[JsonValue] = []

    @override
    def emit(self, record: logging.LogRecord) -> None:
        if "non-JSON create response" in record.getMessage():
            self.subjects.append("malformed_create_reply")


def environment(given: Headers | None) -> None:
    os.environ.clear()
    if given is not None:
        os.environ.update(given)


def request(call: Request) -> JsonValue:
    environment(call.get("environment"))
    params: dict[str, JsonValue] = {
        name: value
        for name, value in (
            ("api_key", call.get("api_key")),
            ("api_base", call.get("api_base")),
            ("base_agent", call.get("base_agent")),
            ("instructions", call.get("instructions")),
            ("base_environment", call.get("base_environment")),
            ("page_size", call.get("page_size")),
            ("page_token", call.get("page_token")),
        )
        if value is not None
    }
    function = call["function"]
    if function == "validate":
        headers = call.get("headers")
        validated = API.validate_environment({} if headers is None else headers, params)
        return {"headers": OBJECT.validate_python(validated)}
    if function == "create_body":
        return {"body": OBJECT.validate_python(API.transform_create_request(call["name"], params))}
    if function == "list_request":
        url, query = API.transform_list_request(call.get("api_base"), params)
    else:
        url, query = API.transform_list_versions_request(call["name"], call.get("api_base"), params)
    pairs: list[JsonValue] = [[name, str(value)] for name, value in query.items()]
    return {"url": url, "query": pairs}


def raw_response(call: Response) -> httpx.Response:
    environment(None)
    return httpx.Response(
        call["status"],
        content=call["body"].encode("utf-8"),
        headers=call.get("headers"),
        request=httpx.Request("POST", "https://example.invalid/v1beta/agents"),
    )


def reply(call: Reply) -> JsonValue:
    response = raw_response(call)
    operation = call["operation"]
    if operation == "list":
        return OBJECT.validate_json(API.transform_list_response(response).model_dump_json())
    if operation == "versions":
        versions = API.transform_list_versions_response(response, call["name"])
        return OBJECT.validate_json(versions.model_dump_json())
    recorder = WarningRecorder()
    logger = logging.getLogger("LiteLLM")
    logger.addHandler(recorder)
    parse = API.transform_create_response if operation == "create" else API.transform_get_response
    try:
        agent = parse(response, call["name"])
    finally:
        logger.removeHandler(recorder)
    return {"agent": OBJECT.validate_json(agent.model_dump_json()), "warnings": recorder.subjects}


def run(input_value: JsonValue) -> JsonValue:
    results: list[JsonValue] = []
    for call in INPUT.validate_python(input_value)["calls"]:
        if call["function"] == "reply":
            results.append(reply(call))
        elif call["function"] == "delete":
            deleted = API.transform_delete_response(raw_response(call), call["name"])
            results.append({"name": deleted.name, "deleted": deleted.deleted})
        else:
            results.append(request(call))
    return {"calls": results}
