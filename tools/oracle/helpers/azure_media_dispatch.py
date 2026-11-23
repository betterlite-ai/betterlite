import base64
import json
from typing import Final, Literal, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.text_to_speech.transformation import AzureAVATextToSpeechConfig
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

import azure_images
from azure_images import clear_ambient
from azure_transcription.exchange import exchange as transcription_exchange
from azure_transcription.probe import Probe as TranscriptionProbe
from openai_transcription.probe import Answer

CLIENT_KEY: Final = "httpx_client" + "ssl_verify_None"

litellm.suppress_debug_info = True


class SharedCase(TypedDict):
    api_base: str
    api_version: str
    model: str
    api_key: str


class GenerationCase(SharedCase):
    case: Literal["image_generation"]
    prompt: str
    options: dict[str, JsonValue]
    reply: JsonValue


class EditCase(SharedCase):
    case: Literal["image_edit"]
    prompt: str
    options: dict[str, JsonValue]


class TranscriptionCase(SharedCase):
    case: Literal["transcription"]
    reply: JsonValue


Voice: TypeAlias = str | dict[str, JsonValue] | None


class SpeechCase(SharedCase):
    case: Literal["openai_speech"]
    input: str
    voice: str
    options: dict[str, JsonValue]


class SsmlCase(SharedCase):
    case: Literal["native_ssml"]
    input: str
    voice: Voice
    options: dict[str, JsonValue]


Call: TypeAlias = GenerationCase | EditCase | SpeechCase | SsmlCase | TranscriptionCase


class Part(TypedDict):
    name: str
    filename: str | None
    content_type: str | None
    body: str


GENERATION: Final[TypeAdapter[GenerationCase]] = TypeAdapter(GenerationCase)
EDIT: Final[TypeAdapter[EditCase]] = TypeAdapter(EditCase)
TRANSCRIPTION: Final[TypeAdapter[TranscriptionCase]] = TypeAdapter(TranscriptionCase)
SPEECH: Final[TypeAdapter[SpeechCase]] = TypeAdapter(SpeechCase)
SSML: Final[TypeAdapter[SsmlCase]] = TypeAdapter(SsmlCase)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
PARTS: Final[TypeAdapter[list[Part]]] = TypeAdapter(list[Part])
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def generation_answer(case: GenerationCase) -> JsonValue:
    step: azure_images.Step = {
        "api_base": case["api_base"],
        "api_version": case["api_version"],
        "model": case["model"],
        "api_key": case["api_key"],
        "prompt": case["prompt"],
        "optional": case["options"],
        "reply_body": json.dumps(case["reply"]),
    }
    view = OBJECT.validate_python(azure_images.generation_step(step))
    headers = OBJECT.validate_python(view["sent_headers"])
    reply = OBJECT.validate_python(view["reply"])
    return {
        "sent_url": view["sent_url"],
        "sent_body": view["sent_body"],
        "api_key": headers["api-key"],
        "authorization": headers["authorization"],
        "created": reply["created"],
        "b64_json": reply["b64_json"],
        "image_url": reply["image_url"],
        "images": reply["images"],
    }


def edit_answer(case: EditCase) -> JsonValue:
    url = OBJECT.validate_python(
        azure_images.url_step(
            {
                "api_base": case["api_base"],
                "api_version": case["api_version"],
                "model": case["model"],
            }
        )
    )
    form = OBJECT.validate_python(
        azure_images.form_step(
            {
                "data": {
                    "model": case["model"],
                    "prompt": case["prompt"],
                    **case["options"],
                },
                "request_url": TEXT.validate_python(url["url"]),
            }
        )
    )
    auth = OBJECT.validate_python(azure_images.auth_step({"api_key": case["api_key"]}))
    fields = OBJECT.validate_python(form["fields"])
    return {
        "url": url["url"],
        "fields": [[name, fields[name]] for name in sorted(fields)],
        "auth": auth["headers"],
    }


def transcription_answer(case: TranscriptionCase) -> JsonValue:
    probe = TranscriptionProbe(
        model=case["model"],
        api_key=case["api_key"],
        api_base=case["api_base"],
        api_version=case["api_version"],
        reply=Answer(
            status=200,
            headers={"content-type": "application/json"},
            body=json.dumps(case["reply"]),
        ),
    )
    view = OBJECT.validate_python(transcription_exchange(probe))
    request = OBJECT.validate_python(view["request"])
    result = OBJECT.validate_python(view["result"])
    parts = PARTS.validate_python(request["parts"])
    positions = [(part["name"], index) for index, part in enumerate(parts)]
    positions.sort()
    entries: list[JsonValue] = []
    for _name, index in positions:
        part = parts[index]
        entries.append(
            {
                "name": part["name"],
                "filename": part["filename"],
                "content_type": part["content_type"],
                "body": base64.b64decode(part["body"]).decode(),
            }
        )
    return {
        "url": request["url"],
        "api_key": request["api_key"],
        "accept": request["accept"],
        "parts": entries,
        "text": result["text"],
        "model": result["model"],
        "provider": result["provider"],
        "duration_recorded": result["duration_recorded"],
    }


class Recorder:
    def __init__(self, body: bytes) -> None:
        self.body = body
        self.seen: list[httpx.Request] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        return httpx.Response(200, content=self.body, request=request)


class SpeechLog(Logging):
    def __init__(self) -> None:
        self.inputs: list[str] = []

    @override
    def pre_call(
        self,
        input: JsonValue,
        api_key: str | None,
        model: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        self.inputs.append(str(input))


class AvaConfig(AzureAVATextToSpeechConfig):
    def __init__(self) -> None:
        object.__init__(self)


def wire_fields(sent: httpx.Request, key_header: str) -> dict[str, JsonValue]:
    headers: dict[str, str] = dict(sent.headers)
    return {
        "method": sent.method, "url": str(sent.url),
        "api_key": headers.get(key_header), "authorization": headers.get("authorization"),
        "accept": headers.get("accept"), "output_format": headers.get("x-microsoft-outputformat"),
    }


def openai_answer(case: SpeechCase) -> JsonValue:
    clear_ambient()
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    recorder = Recorder(b"ID3")
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(recorder.respond))
    options: dict[str, JsonValue] = dict(case["options"])
    response = AzureChatCompletion().audio_speech(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1362-L1426
        model=case["model"], input=case["input"], voice=case["voice"],
        optional_params=options, api_key=case["api_key"], api_base=case["api_base"],
        api_version=case["api_version"], organization=None, max_retries=2,
        timeout=600.0, logging_obj=SpeechLog(), aspeech=False, litellm_params=None,
    )
    sent = recorder.seen[0]
    return {
        **wire_fields(sent, "api-key"),
        "body": JSON.validate_json(sent.content),
        "status": response.response.status_code,
        "content": response.response.content.decode(),
    }


def native_answer(case: SsmlCase) -> JsonValue:
    clear_ambient()
    recorder = Recorder(b"AUDIO")
    handler = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(recorder.respond)))
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    litellm.in_memory_llm_clients_cache.set_cache(key=CLIENT_KEY, value=handler)  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/caching/llm_caching_handler.py#L58
    options: dict[str, JsonValue] = dict(case["options"])
    no_params: dict[str, JsonValue] = {}
    answered = AvaConfig().dispatch_text_to_speech(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L108-L134
        model=case["model"], input=case["input"], voice=case["voice"],
        optional_params=options, litellm_params_dict=no_params,
        logging_obj=SpeechLog(), timeout=600.0, extra_headers=None,
        base_llm_http_handler=BaseLLMHTTPHandler(), aspeech=False,
        api_base=case["api_base"], api_key=case["api_key"],
    )
    response = cast("HttpxBinaryResponseContent", answered)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L108-L134
    sent = recorder.seen[0]
    fields = wire_fields(sent, "ocp-apim-subscription-key")
    fields["body"] = sent.content.decode()
    fields["status"] = response.response.status_code
    fields["content"] = response.response.content.decode()
    return fields


def run(input_value: Call) -> JsonValue:
    if input_value["case"] == "image_generation":
        return generation_answer(GENERATION.validate_python(input_value))
    if input_value["case"] == "image_edit":
        return edit_answer(EDIT.validate_python(input_value))
    if input_value["case"] == "openai_speech":
        return openai_answer(SPEECH.validate_python(input_value))
    if input_value["case"] == "native_ssml":
        return native_answer(SSML.validate_python(input_value))
    return transcription_answer(TRANSCRIPTION.validate_python(input_value))
