from datetime import UTC, datetime
from typing import Literal, NotRequired, TypeAlias, TypedDict

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.gemini.interactions.transformation import GoogleAIStudioInteractionsConfig
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue, TypeAdapter

Json: TypeAlias = JsonValue
Object: TypeAlias = dict[str, Json]
Headers: TypeAlias = dict[str, str]


class TransformRequest(TypedDict):
    function: Literal["transform_request"]
    model: NotRequired[str]
    agent: NotRequired[str]
    input: NotRequired[str]
    optional_params: Object


class Lifecycle(TypedDict):
    function: Literal["lifecycle"]
    action: Literal["get", "delete", "cancel"]
    interaction_id: str
    api_base: str


class Response(TypedDict):
    function: Literal["response"]
    action: Literal["create", "get"]
    status: int
    headers: NotRequired[Headers]
    body: str


class Delete(TypedDict):
    function: Literal["response"]
    action: Literal["delete"]
    interaction_id: str
    status: int
    headers: NotRequired[Headers]
    body: str


Call: TypeAlias = TransformRequest | Lifecycle | Response | Delete


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
NO_HEADERS: Headers = {}
KEY = GenericLiteLLMParams(api_key="key")


def resolve(call: Call) -> Json:
    if call["function"] == "transform_request":
        body: Object = CONFIG.transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L161-L169
            call.get("model"),
            call.get("agent"),
            call.get("input"),
            call["optional_params"],
            GenericLiteLLMParams(),
            NO_HEADERS,
        )
        return {"body": body}
    if call["function"] == "lifecycle":
        build = {  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
            "get": CONFIG.transform_get_interaction_request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
            "delete": CONFIG.transform_delete_interaction_request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
            "cancel": CONFIG.transform_cancel_interaction_request,  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        }[call["action"]]
        url, _body = build(call["interaction_id"], call["api_base"], KEY, NO_HEADERS)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/gemini/interactions/transformation.py#L122-L389
        return {"url": url}
    reply = httpx.Response(
        call["status"], headers=call.get("headers"), content=call["body"].encode("utf-8")
    )
    if call["action"] == "delete":
        deleted = CONFIG.transform_delete_interaction_response(
            reply, LOGGING, call["interaction_id"]
        )
        return OBJECT.validate_python(deleted.model_dump())  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1057-L1142
    if call["action"] == "get":
        interaction = CONFIG.transform_get_interaction_response(reply, LOGGING)
    else:
        interaction = CONFIG.transform_response(None, reply, LOGGING)
    hidden: Object = interaction._hidden_params  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1082
    return {
        "record": OBJECT.validate_python(interaction.model_dump()),  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/interactions/generated.py#L1057-L1142
        "headers": hidden.get("headers"),
        "additional_headers": hidden.get("additional_headers"),
    }


def run(input_value: Json) -> Json:
    parsed = INPUT.validate_python(input_value)
    return {"calls": [resolve(call) for call in parsed["calls"]]}
