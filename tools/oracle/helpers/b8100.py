from datetime import UTC, datetime
from typing import Final, Literal, Protocol, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.a2a.chat.streaming_iterator import A2AModelResponseIterator
from litellm.llms.a2a.chat.transformation import A2AConfig
from litellm.llms.a2a.common_utils import A2AError
from litellm.proxy.agent_endpoints.agent_registry import global_agent_registry
from litellm.types.agents import AgentResponse
from litellm.types.llms.openai import AllMessageValues
from litellm.types.utils import GenericStreamingChunk, LlmProviders, ModelResponse, ModelResponseStream
from litellm.utils import ProviderConfigManager
from pydantic import BaseModel, ConfigDict, JsonValue, TypeAdapter

Body: TypeAlias = dict[str, JsonValue]

# a plain literal is the only message shape pydantic leaves as dicts, and LiteLLM reads the prompt from them
PROMPT: list[AllMessageValues] = [
    {"role": "user", "content": "hello there"},
    {"role": "user", "content": "second"},
]
EMPTY_MESSAGES: list[AllMessageValues] = []
EMPTY: Body = {}


class ChunkView(TypedDict):
    text: str
    is_finished: bool
    finish_reason: str
    index: int


class ReplyMessage(TypedDict):
    role: str
    content: str | None


class ReplyChoice(TypedDict):
    index: int
    finish_reason: str
    message: ReplyMessage


class ReplyView(TypedDict):
    id: str
    model: str | None
    choices: list[ReplyChoice]


class Failure(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    status_code: int | None = None
    message: str | None = None


class ChatConfig(Protocol):
    def map_openai_params(
        self, non_default_params: Body, optional_params: Body, model: str, drop_params: bool
    ) -> Body: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        messages: list[AllMessageValues],
        optional_params: Body,
        litellm_params: Body,
        api_key: str | None = None,
        api_base: str | None = None,
    ) -> Body: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: Body,
        litellm_params: Body,
        stream: bool | None = None,
    ) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[AllMessageValues],
        optional_params: Body,
        litellm_params: Body,
        headers: Body,
    ) -> Body: ...

    def transform_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: ModelResponse,
        logging_obj: Logging,
        request_data: Body,
        messages: list[AllMessageValues],
        optional_params: Body,
        litellm_params: Body,
        encoding: None,
        api_key: str | None = None,
        json_mode: bool | None = None,
    ) -> ModelResponse: ...


class StreamParser(Protocol):
    def chunk_parser(self, chunk: Body) -> GenericStreamingChunk | ModelResponseStream: ...


class AgentCase(TypedDict):
    name: str
    card: Body
    params: Body


class ProbeCase(TypedDict):
    agent_name: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str] | None
    optional_params: Body


class RegistryInput(TypedDict):
    op: Literal["registry"]
    agents: list[AgentCase]
    probes: list[ProbeCase]


class RequestCase(TypedDict):
    model: str
    api_base: str | None
    api_key: str | None
    headers: dict[str, str]
    non_default_params: Body
    optional_params: Body


class RequestInput(TypedDict):
    op: Literal["request"]
    cases: list[RequestCase]


class EmptyInput(TypedDict):
    op: Literal["request_empty"]


class ResponseCase(TypedDict):
    model: str
    status: int
    body: str


class ResponseInput(TypedDict):
    op: Literal["response"]
    cases: list[ResponseCase]


class StreamCase(TypedDict):
    chunk: Body


class StreamInput(TypedDict):
    op: Literal["stream"]
    chunks: list[StreamCase]


Input = RegistryInput | RequestInput | EmptyInput | ResponseInput | StreamInput

CHUNK_VIEW: Final[TypeAdapter[ChunkView]] = TypeAdapter(ChunkView)


def config() -> ChatConfig:
    found = ProviderConfigManager.get_provider_chat_config(
        model="my-agent", provider=LlmProviders.A2A
    )
    if found is None:
        raise ValueError("no A2A chat config")
    return found


def stream_parser() -> StreamParser:
    return A2AModelResponseIterator(iter(()), sync_stream=False)


def logging_object() -> Logging:
    started = datetime(2026, 1, 1, tzinfo=UTC)
    return Logging(
        model="my-agent",
        messages=None,
        stream=False,
        call_type="completion",
        start_time=started,
        litellm_call_id="call-1",
        function_id="fn-1",
    )


def register(agent: AgentCase) -> None:
    found = AgentResponse.model_validate(
        {
            "agent_id": agent["name"],
            "agent_name": agent["name"],
            "agent_card_params": agent["card"],
            "litellm_params": agent["params"],
        }
    )
    global_agent_registry.register_agent(found)


def probe_view(probe: ProbeCase) -> JsonValue:
    optional_params: Body = dict(probe["optional_params"])
    resolved: tuple[str | None, str | None, Body | None] = (
        A2AConfig.resolve_agent_config_from_registry(
            probe["agent_name"],
            probe["api_base"],
            probe["api_key"],
            probe["headers"],
            optional_params,
        )
    )
    api_base, api_key, headers = resolved
    return {
        "api_base": api_base,
        "api_key": api_key,
        "headers": headers,
        "optional_params": optional_params,
    }


def registry(input_value: RegistryInput) -> JsonValue:
    for agent in input_value["agents"]:
        register(agent)
    return {"probes": [probe_view(probe) for probe in input_value["probes"]]}


def request_view(chat: ChatConfig, case: RequestCase) -> JsonValue:
    optional_params: Body = dict(case["optional_params"])
    chat.map_openai_params(
        non_default_params=case["non_default_params"],
        optional_params=optional_params,
        model=case["model"],
        drop_params=False,
    )
    headers: Body = chat.validate_environment(
        headers=case["headers"],
        model=case["model"],
        messages=EMPTY_MESSAGES,
        optional_params=optional_params,
        litellm_params=EMPTY,
        api_key=case["api_key"],
        api_base=case["api_base"],
    )
    url = chat.get_complete_url(
        api_base=case["api_base"],
        api_key=case["api_key"],
        model=case["model"],
        optional_params=optional_params,
        litellm_params=EMPTY,
        stream=None,
    )
    body: Body = chat.transform_request(
        model=case["model"],
        messages=PROMPT,
        optional_params=optional_params,
        litellm_params=EMPTY,
        headers=headers,
    )
    return {"url": url, "headers": headers, "body": body}


def request(input_value: RequestInput) -> JsonValue:
    chat = config()
    return {"cases": [request_view(chat, case) for case in input_value["cases"]]}


def empty_request() -> JsonValue:
    chat = config()
    try:
        chat.transform_request(
            model="my-agent",
            messages=[],
            optional_params=EMPTY,
            litellm_params=EMPTY,
            headers=EMPTY,
        )
    except ValueError as error:
        return {"raised": str(error)}
    return {"raised": None}


def raw_response(case: ResponseCase) -> httpx.Response:
    return httpx.Response(
        case["status"],
        content=case["body"].encode(),
        headers={"content-type": "application/json"},
        request=httpx.Request("POST", "http://a2a.test/agent"),
    )


def reply_view(reply: ModelResponse) -> JsonValue:
    fields: ReplyView = TypeAdapter(ReplyView).validate_json(reply.model_dump_json())
    choice = fields["choices"][0]
    return {
        "id": fields["id"],
        "model": fields["model"],
        "index": choice["index"],
        "finish_reason": choice["finish_reason"],
        "role": choice["message"]["role"],
        "content": choice["message"]["content"],
    }


def response_view(chat: ChatConfig, case: ResponseCase) -> JsonValue:
    try:
        reply = chat.transform_response(
            model=case["model"],
            raw_response=raw_response(case),
            model_response=ModelResponse(),
            logging_obj=logging_object(),
            request_data=EMPTY,
            messages=EMPTY_MESSAGES,
            optional_params=EMPTY,
            litellm_params=EMPTY,
            encoding=None,
        )
    except A2AError as error:
        failure = Failure.model_validate(error)
        return {"error": {"status": failure.status_code, "message": failure.message}}
    return {"reply": reply_view(reply)}


def response(input_value: ResponseInput) -> JsonValue:
    chat = config()
    return {"cases": [response_view(chat, case) for case in input_value["cases"]]}


def chunk_view(value: Body) -> JsonValue:
    try:
        found = CHUNK_VIEW.validate_python(stream_parser().chunk_parser(value))
    except A2AError as error:
        failure = Failure.model_validate(error)
        return {"error": {"status": failure.status_code, "message": failure.message}}
    return {
        "chunk": {
            "text": found["text"],
            "is_finished": found["is_finished"],
            "finish_reason": found["finish_reason"],
            "index": found["index"],
        }
    }


def stream(input_value: StreamInput) -> JsonValue:
    return {"chunks": [chunk_view(case["chunk"]) for case in input_value["chunks"]]}


def run(input_value: Input) -> JsonValue:
    if input_value["op"] == "registry":
        return registry(input_value)
    if input_value["op"] == "request":
        return request(input_value)
    if input_value["op"] == "request_empty":
        return empty_request()
    if input_value["op"] == "response":
        return response(input_value)
    return stream(input_value)
