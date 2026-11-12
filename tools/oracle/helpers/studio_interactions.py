import os
from datetime import UTC, datetime
from typing import Annotated, Literal, NotRequired, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.interactions.transformation import GoogleAIStudioInteractionsConfig
from litellm.types.interactions import InteractionsAPIResponse
from litellm.types.router import GenericLiteLLMParams
from pydantic import Field, JsonValue, TypeAdapter

Json: TypeAlias = JsonValue
Object: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, str]


class TransformRequest(TypedDict):
    function: Literal["transform_request"]
    model: NotRequired[str]
    agent: NotRequired[str]
    input: NotRequired[str]
    optional_params: Object


class ValidateEnvironment(TypedDict):
    function: Literal["validate_environment"]
    headers: NotRequired[Headers]
    api_key: NotRequired[str]
    with_params: bool
    env: NotRequired[Headers]


class CreateUrl(TypedDict):
    function: Literal["create_url"]
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    stream: NotRequired[bool]
    env: NotRequired[Headers]


class Lifecycle(TypedDict):
    function: Literal["lifecycle"]
    action: Literal["get", "delete", "cancel"]
    interaction_id: str
    api_base: NotRequired[str]
    api_key: NotRequired[str]
    env: NotRequired[Headers]


class Response(TypedDict):
    function: Literal["response"]
    action: Literal["create", "get", "cancel"]
    status: int
    headers: NotRequired[Headers]
    body: str


class Delete(TypedDict):
    function: Literal["delete"]
    status: int
    headers: NotRequired[Headers]
    body: str
    interaction_id: str


class StreamChunk(TypedDict):
    function: Literal["stream_chunk"]
    chunk: Object


Call: TypeAlias = Annotated[
    TransformRequest
    | ValidateEnvironment
    | CreateUrl
    | Lifecycle
    | Response
    | Delete
    | StreamChunk,
    Field(discriminator="function"),
]


class Input(TypedDict):
    calls: list[Call]


INPUT = TypeAdapter(Input)
OBJECT = TypeAdapter(Object)
CONFIG = GoogleAIStudioInteractionsConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L88
LOGGING = Logging(
    model="gemini-2.5-flash",
    messages=[],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/litellm_core_utils/litellm_logging.py#L576
    stream=False,
    call_type="completion",
    start_time=datetime(2026, 1, 1, tzinfo=UTC),
    litellm_call_id="studio-interactions",
    function_id="studio-interactions",
)


def use_environment(env: Headers | None) -> None:
    os.environ.clear()
    os.environ.update({} if env is None else env)


def hidden_headers(reply: InteractionsAPIResponse) -> Object:
    hidden: Object = reply._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1082
    return {
        "headers": hidden.get("headers"),
        "additional_headers": hidden.get("additional_headers"),
    }


def transform_request(call: TransformRequest) -> Json:
    body: Object = CONFIG.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L161-L169
        call.get("model"),
        call.get("agent"),
        call.get("input"),
        call["optional_params"],
        GenericLiteLLMParams(),
        {},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
    )
    return {"body": body}


def validate_environment(call: ValidateEnvironment) -> Json:
    use_environment(call.get("env"))
    litellm_params = (
        GenericLiteLLMParams(api_key=call.get("api_key")) if call["with_params"] else None
    )
    headers: Headers = CONFIG.validate_environment(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L127
        dict(call["headers"]) if "headers" in call else {},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        "gemini/gemini-2.5-flash",
        litellm_params,
    )
    return {"headers": Object(headers)}


def create_url(call: CreateUrl) -> Json:
    use_environment(call.get("env"))
    url = CONFIG.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L140-L146
        call.get("api_base"),
        None,
        None,
        {"api_key": call.get("api_key")},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        call.get("stream"),
    )
    return {"url": url}


def lifecycle(call: Lifecycle) -> Json:
    use_environment(call.get("env"))
    litellm_params = GenericLiteLLMParams(api_key=call.get("api_key"))
    api_base = ""
    if "api_base" in call:
        api_base = call["api_base"]
    interaction_id = call["interaction_id"]
    if call["action"] == "get":
        request = CONFIG.transform_get_interaction_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L294-L300
            interaction_id,
            api_base,
            litellm_params,
            {},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        )
    elif call["action"] == "delete":
        request = CONFIG.transform_delete_interaction_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L328-L334
            interaction_id,
            api_base,
            litellm_params,
            {},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        )
    else:
        request = CONFIG.transform_cancel_interaction_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L359-L365
            interaction_id,
            api_base,
            litellm_params,
            {},  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        )
    url, body = request  # type: ignore[misc]  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L294-L300
    return {"url": url, "body": Object(body)}  # type: ignore[misc]  # pyright: ignore[reportUnknownArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L294-L300


def response(call: Response) -> Json:
    reply = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode("utf-8")
    )
    if call["action"] == "cancel":
        cancelled = CONFIG.transform_cancel_interaction_response(reply, LOGGING)
        return {"record": OBJECT.validate_python(cancelled.model_dump())}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1057-L1142
    if call["action"] == "get":
        interaction = CONFIG.transform_get_interaction_response(reply, LOGGING)
    else:
        interaction = CONFIG.transform_response(None, reply, LOGGING)
    return {
        "record": OBJECT.validate_python(interaction.model_dump()),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1057-L1142
        **hidden_headers(interaction),
    }


def delete(call: Delete) -> Json:
    reply = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode("utf-8")
    )
    result = CONFIG.transform_delete_interaction_response(reply, LOGGING, call["interaction_id"])
    return OBJECT.validate_python(result.model_dump())  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1057-L1142


def stream_chunk(call: StreamChunk) -> Json:
    record = CONFIG.transform_streaming_response(None, call["chunk"], LOGGING)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L282-L286
    return {"record": OBJECT.validate_python(record.model_dump())}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1057-L1142


def resolve(call: Call) -> Json:
    result: Json
    if call["function"] == "transform_request":
        result = transform_request(call)
    elif call["function"] == "validate_environment":
        result = validate_environment(call)
    elif call["function"] == "create_url":
        result = create_url(call)
    elif call["function"] == "lifecycle":
        result = lifecycle(call)
    elif call["function"] == "response":
        result = response(call)
    elif call["function"] == "delete":
        result = delete(call)
    else:
        result = stream_chunk(call)
    return result


def run(input_value: Json) -> Json:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
