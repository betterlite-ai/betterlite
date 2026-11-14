import json
from typing import Literal, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.llms.vertex_ai.text_to_speech.transformation import (
    VertexAILyriaTextToSpeechConfig,
)
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from openai import BadRequestError
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

PROJECT = "project-1"
Voice: TypeAlias = str | dict[str, JsonValue] | None
Params: TypeAlias = dict[str, JsonValue]
FAILED: JsonValue = {"error": True}


class Reply(Protocol):
    response: httpx.Response
    content: bytes


class Config(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self, model: str, optional_params: Params, **kwargs: JsonValue
    ) -> tuple[str | None, Params]: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: Params
    ) -> str: ...

    def validate_environment(
        self, headers: Params, model: str, **kwargs: JsonValue
    ) -> Params: ...

    def transform_text_to_speech_request(  # ast-grep-ignore: no-long-names
        self, **kwargs: JsonValue
    ) -> Params: ...

    def transform_text_to_speech_response(  # ast-grep-ignore: no-long-names
        self, model: str, raw_response: httpx.Response, logging_obj: JsonValue
    ) -> Reply: ...


class OracleConfig(VertexAILyriaTextToSpeechConfig):  # ast-grep-ignore: no-long-names
    @override
    def get_access_token(
        self,
        credentials: VERTEX_CREDENTIALS_TYPES | None,
        project_id: str | None,
        _retry_reauth: bool = False,
    ) -> tuple[str, str]:
        del credentials, _retry_reauth
        return "oracle-token", PROJECT if project_id is None else project_id


CONFIG = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/text_to_speech/transformation.py#L487-L676
    Config, OracleConfig()
)

MAPPED = TypeAdapter(Params)


class MappingCase(TypedDict):
    model: str
    params: Params
    voice: Voice
    drop_params: bool
    global_drop_params: bool


class PreparedCase(TypedDict):
    model: str
    params: Params
    voice: Voice
    drop_params: bool
    global_drop_params: bool
    project: str | None
    location: str | None
    api_base: str | None
    input: str
    headers: Params


class DecodedCase(TypedDict):
    model: str
    status: int
    body: str


class MappedInput(TypedDict):
    function: Literal["mapped"]
    cases: list[MappingCase]


class PreparedInput(TypedDict):
    function: Literal["prepared"]
    cases: list[PreparedCase]


class DecodedInput(TypedDict):
    function: Literal["decoded"]
    cases: list[DecodedCase]


Input: TypeAlias = MappedInput | PreparedInput | DecodedInput


def mapping_view(case: MappingCase) -> JsonValue:
    litellm.drop_params = case["global_drop_params"]
    try:
        name, params = CONFIG.map_openai_params(
            case["model"],
            case["params"],
            voice=case["voice"],
            drop_params=case["drop_params"],
            kwargs={},
        )
    except (ValueError, BadRequestError):
        return FAILED
    else:
        return {
            "supported": list[JsonValue](
                CONFIG.get_supported_openai_params(case["model"])
            ),
            "voice": name,
            "params": MAPPED.validate_json(json.dumps(params)),
        }


def prepared_view(case: PreparedCase) -> JsonValue:
    litellm.drop_params = case["global_drop_params"]
    litellm_params: Params = {}
    if case["project"] is not None:
        litellm_params["vertex_project"] = case["project"]
    if case["location"] is not None:
        litellm_params["vertex_location"] = case["location"]
    try:
        name, optional = CONFIG.map_openai_params(
            case["model"],
            case["params"],
            voice=case["voice"],
            drop_params=case["drop_params"],
            kwargs={},
        )
        url = CONFIG.get_complete_url(case["model"], case["api_base"], litellm_params)
        headers = CONFIG.validate_environment(
            case["headers"], case["model"], api_base=case["api_base"]
        )
        data = CONFIG.transform_text_to_speech_request(
            model=case["model"],
            input=case["input"],
            voice=name,
            optional_params=optional,
            litellm_params=litellm_params,
            headers=headers,
        )
    except (ValueError, BadRequestError):
        return FAILED
    else:
        return {
            "url": url,
            "body": MAPPED.validate_json(json.dumps(data.get("dict_body"))),
            "headers": MAPPED.validate_json(json.dumps(data.get("headers"))),
        }


def audio_view(case: DecodedCase) -> JsonValue:
    raw = httpx.Response(status_code=case["status"], content=case["body"].encode())
    try:
        reply = CONFIG.transform_text_to_speech_response(case["model"], raw, None)
    except (json.JSONDecodeError, AttributeError, TypeError, ValueError):
        return FAILED
    headers = reply.response.headers
    return {
        "status": reply.response.status_code,
        "contentType": next(iter(headers.get_list("content-type")), None),
        "bytes": list[JsonValue](reply.content),
    }


def run(input_value: Input) -> JsonValue:
    match input_value["function"]:
        case "mapped":
            return {"cases": [mapping_view(case) for case in input_value["cases"]]}
        case "prepared":
            return {"cases": [prepared_view(case) for case in input_value["cases"]]}
        case "decoded":
            return {"cases": [audio_view(case) for case in input_value["cases"]]}
