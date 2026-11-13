import logging
import os
from typing import Final, Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.llms.gemini.agents.transformation import GeminiAgentsConfig
from litellm.types.agents import AgentCreateResponse, AgentListResponse, AgentVersionsResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

litellm.suppress_debug_info = True

Headers: TypeAlias = dict[str, str]


class Validate(TypedDict):
    function: Literal["validate"]
    headers: NotRequired[Headers]
    api_key: NotRequired[str]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]


class CreateBody(TypedDict):
    function: Literal["create_body"]
    name: str
    params: NotRequired[dict[str, JsonValue]]


class ListRequest(TypedDict):
    function: Literal["list_request"]
    api_base: NotRequired[str]
    environment: NotRequired[Headers]
    page_size: NotRequired[int]
    page_token: NotRequired[str]


class VersionsRequest(TypedDict):
    function: Literal["versions_request"]
    name: str
    api_base: NotRequired[str]
    environment: NotRequired[Headers]
    page_size: NotRequired[int]
    page_token: NotRequired[str]


class ItemRequest(TypedDict):
    function: Literal["item_request"]
    name: str
    api_base: NotRequired[str]
    environment: NotRequired[Headers]


class Reply(TypedDict):
    function: Literal["reply"]
    operation: Literal["create", "get", "list", "versions", "delete"]
    name: str
    status: int
    body: str
    headers: NotRequired[Headers]


Call: TypeAlias = (
    Validate | CreateBody | ListRequest | VersionsRequest | ItemRequest | Reply
)


class Input(TypedDict):
    calls: list[Call]


class AgentFields(TypedDict):
    id: str | None
    name: str | None


class AgentPageFields(TypedDict):
    agents: list[JsonValue]
    next_page_token: str | None


class AgentVersionsFields(TypedDict):
    agent_versions: list[JsonValue]
    next_page_token: str | None


INPUT: Final = TypeAdapter(Input)
OBJECT: Final = TypeAdapter(dict[str, JsonValue])
QUERY: Final = TypeAdapter(dict[str, JsonValue])
AGENT: Final = TypeAdapter(AgentFields)
PAGE: Final = TypeAdapter(AgentPageFields)
VERSIONS: Final = TypeAdapter(AgentVersionsFields)

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


def litellm_params(call: Validate) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    api_key = call.get("api_key")
    if api_key is not None:
        params["api_key"] = api_key
    api_base = call.get("api_base")
    if api_base is not None:
        params["api_base"] = api_base
    return params


def page_params(call: ListRequest | VersionsRequest) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {}
    page_size = call.get("page_size")
    if page_size is not None:
        params["page_size"] = page_size
    page_token = call.get("page_token")
    if page_token is not None:
        params["page_token"] = page_token
    return params


def query_pairs(query: dict[str, JsonValue]) -> list[JsonValue]:
    return [[name, str(value)] for name, value in query.items()]


def target_value(url: str, query: dict[str, JsonValue]) -> JsonValue:
    return {"url": url, "query": query_pairs(query)}


def agent_value(agent: AgentCreateResponse, warnings: list[JsonValue]) -> JsonValue:
    fields = AGENT.validate_json(agent.model_dump_json())
    extension = OBJECT.validate_json(agent.model_dump_json(exclude={"id", "name"}))
    return {
        "agent": {"id": fields["id"], "name": fields["name"], "extension": extension},
        "warnings": warnings,
    }


def page_value(page: AgentListResponse) -> JsonValue:
    fields = PAGE.validate_json(page.model_dump_json())
    return {"agents": fields["agents"], "next_page_token": fields["next_page_token"]}


def versions_value(page: AgentVersionsResponse) -> JsonValue:
    fields = VERSIONS.validate_json(page.model_dump_json())
    return {
        "agent_versions": fields["agent_versions"],
        "next_page_token": fields["next_page_token"],
    }


def validate(call: Validate) -> JsonValue:
    environment(call.get("environment"))
    given = call.get("headers")
    headers: Headers = {}
    if given is not None:
        headers.update(given)
    validated = API.validate_environment(headers, litellm_params(call))
    return {"headers": OBJECT.validate_python(validated)}


def create_body(call: CreateBody) -> JsonValue:
    params = call.get("params")
    given: dict[str, JsonValue] = {}
    if params is not None:
        given.update(params)
    return {"body": OBJECT.validate_python(API.transform_create_request(call["name"], given))}


def list_request(call: ListRequest) -> JsonValue:
    environment(call.get("environment"))
    url, query = API.transform_list_request(call.get("api_base"), page_params(call))
    return target_value(url, QUERY.validate_python(query))


def versions_request(call: VersionsRequest) -> JsonValue:
    environment(call.get("environment"))
    url, query = API.transform_list_versions_request(
        call["name"], call.get("api_base"), page_params(call)
    )
    return target_value(url, QUERY.validate_python(query))


def item_request(call: ItemRequest) -> JsonValue:
    environment(call.get("environment"))
    params: dict[str, JsonValue] = {}
    get_url, query = API.transform_get_request(call["name"], call.get("api_base"), params)
    delete_url = API.transform_delete_request(call["name"], call.get("api_base"), params)
    return {
        "url": get_url,
        "delete_url": delete_url,
        "query": query_pairs(QUERY.validate_python(query)),
    }


def reply(call: Reply) -> JsonValue:
    environment(None)
    request = httpx.Request("POST", "https://example.invalid/v1beta/agents")
    response = httpx.Response(
        call["status"],
        content=call["body"].encode("utf-8"),
        headers=call.get("headers"),
        request=request,
    )
    operation = call["operation"]
    if operation == "delete":
        deleted = API.transform_delete_response(response, call["name"])
        return {"name": deleted.name, "deleted": deleted.deleted}
    if operation == "create":
        recorder = WarningRecorder()
        logger = logging.getLogger("LiteLLM")
        logger.addHandler(recorder)
        try:
            record = API.transform_create_response(response, call["name"])
        finally:
            logger.removeHandler(recorder)
        return agent_value(record, recorder.subjects)
    if operation == "get":
        record = API.transform_get_response(response, call["name"])
        return agent_value(record, [])
    if operation == "list":
        return page_value(API.transform_list_response(response))
    return versions_value(API.transform_list_versions_response(response, call["name"]))


def resolve(call: Call) -> JsonValue:
    if call["function"] == "validate":
        return validate(call)
    if call["function"] == "create_body":
        return create_body(call)
    if call["function"] == "list_request":
        return list_request(call)
    if call["function"] == "versions_request":
        return versions_request(call)
    if call["function"] == "item_request":
        return item_request(call)
    return reply(call)


def run(input_value: JsonValue) -> JsonValue:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
