import asyncio
import os
from collections.abc import Coroutine
from typing import Final, NotRequired, TypedDict, cast

import httpx
import litellm
import litellm.llms.azure.azure as azure_module
import openai
from litellm.caching.llm_caching_handler import LLMClientCache
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.azure.azure import AzureChatCompletion
from litellm.llms.azure.text_to_speech.transformation import AzureAVATextToSpeechConfig
from litellm.llms.custom_httpx.llm_http_handler import BaseLLMHTTPHandler
from litellm.types.llms.openai import HttpxBinaryResponseContent
from openai import AzureOpenAI
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

from azure_images import clear_ambient

litellm.suppress_debug_info = True

CONFIG: Final[AzureAVATextToSpeechConfig] = AzureAVATextToSpeechConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L34


class Reply(TypedDict):
    status: int
    headers: NotRequired[dict[str, str]]
    body: NotRequired[str]


class AudioSpeech(TypedDict):
    model: str
    input: str
    voice: str
    api_base: str
    api_version: str
    reply: Reply
    api_key: NotRequired[str | None]
    optional_params: NotRequired[dict[str, JsonValue]]
    aspeech: NotRequired[bool]


class NativeCall(TypedDict):
    model: str
    input: str
    timeout_seconds: int
    voice: NotRequired[JsonValue]
    optional_params: NotRequired[dict[str, JsonValue]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    extra_headers: NotRequired[dict[str, str] | None]
    aspeech: NotRequired[bool]
    globals: NotRequired[dict[str, str]]
    environment: NotRequired[dict[str, str]]


class ProbeScenario(TypedDict):
    outcome: str
    api_key: str
    api_base: str
    api_version: str
    model: NotRequired[str | None]
    mode: NotRequired[str]
    messages: NotRequired[list[JsonValue]]
    reply: NotRequired[Reply]


AUDIO: Final[TypeAdapter[AudioSpeech]] = TypeAdapter(AudioSpeech)
OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
TEXT: Final[TypeAdapter[str]] = TypeAdapter(str)
SECONDS: Final[TypeAdapter[float | int]] = TypeAdapter(float | int)


class Recorder:
    def __init__(self, reply: Reply) -> None:
        self.status = reply["status"]
        self.headers = reply.get("headers")
        body = reply.get("body")
        self.body = "" if body is None else body
        self.seen: list[httpx.Request] = []

    def respond(self, request: httpx.Request) -> httpx.Response:
        self.seen.append(request)
        return httpx.Response(
            self.status, headers=self.headers, content=self.body.encode(), request=request
        )


class SpeechLog(Logging):
    def __init__(self) -> None:
        self.pre: dict[str, JsonValue] | None = None

    @override
    def pre_call(
        self,
        input: JsonValue,
        api_key: str | None,
        model: str | None = None,
        additional_args: dict[str, JsonValue] | None = None,
    ) -> None:
        if additional_args is None:
            raise ValueError("the speech pre-call carries additional args")
        args = OBJECT.validate_python(additional_args)
        self.pre = {
            "input": TEXT.validate_python(input),
            "api_base": TEXT.validate_python(args["api_base"]),
            "data": args["complete_input_dict"],
        }


def wire_view(sent: httpx.Request) -> JsonValue:
    headers = dict(sent.headers)
    return {
        "method": sent.method, "url": str(sent.url), "api_key": headers.get("api-key"),
        "authorization": headers.get("authorization"), "accept": headers.get("accept"),
        "x_extra": headers.get("x-extra"),
        "body": None if not sent.content else JSON.validate_json(sent.content),
    }


def audio_view(case: AudioSpeech) -> JsonValue:
    clear_ambient()
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    log = SpeechLog()
    recorder = Recorder(case["reply"])
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(recorder.respond))
    litellm.aclient_session = httpx.AsyncClient(transport=httpx.MockTransport(recorder.respond))
    aspeech = case.get("aspeech")
    raw_options = case.get("optional_params")
    options: dict[str, JsonValue] = {} if raw_options is None else dict(raw_options)
    error: openai.APIStatusError | None = None
    status = 0
    content = b""
    try:
        answered = AzureChatCompletion().audio_speech(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1362-L1426
            model=case["model"], input=case["input"], voice=case["voice"], aspeech=aspeech,
            optional_params=options, api_key=case.get("api_key"), api_base=case["api_base"],
            api_version=case["api_version"], organization=None, max_retries=2,
            timeout=600.0, logging_obj=log, litellm_params=None,
        )
        if aspeech is True:
            pending = cast("Coroutine[object, object, HttpxBinaryResponseContent]", answered)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1362-L1426
            response = asyncio.run(pending)
        else:
            response = answered
        status = response.response.status_code
        content = response.response.content
    except openai.APIStatusError as caught:
        error = caught
    return {
        "request": None if not recorder.seen else wire_view(recorder.seen[0]),
        "pre_call": log.pre,
        "result": None if error is not None else {"status": status, "content": content.decode()},
        "error": None if error is None else {"status": error.status_code},
    }


class RecordingHandler:
    def __init__(self) -> None:
        self.calls: list[dict[str, JsonValue]] = []

    def text_to_speech_handler(self, **kwargs: JsonValue) -> str:
        self.calls.append({
            "voice": kwargs["voice"], "extra_headers": kwargs["extra_headers"],
            "litellm_params": OBJECT.validate_python(kwargs["litellm_params"]),
            "optional_params": OBJECT.validate_python(kwargs["text_to_speech_optional_params"]),
            "is_async": kwargs["_is_async"], "client": kwargs["client"],
            "timeout": str(int(SECONDS.validate_python(kwargs["timeout"]))),
            "provider": TEXT.validate_python(kwargs["custom_llm_provider"]),
            "result": "handler-answer", "error": None,
        })
        return "handler-answer"


def native_view(call: NativeCall) -> JsonValue:
    clear_ambient()
    globals_value = call.get("globals")
    speech_globals = {} if globals_value is None else globals_value
    litellm.api_base = speech_globals.get("api_base")
    litellm.api_key = speech_globals.get("api_key")
    litellm.azure_key = speech_globals.get("azure_key")
    environment_value = call.get("environment")
    environment = {} if environment_value is None else environment_value
    for name, value in environment.items():
        os.environ[name] = value
    raw_optional = call.get("optional_params")
    raw_params = call.get("litellm_params")
    optional_params: dict[str, JsonValue] = {} if raw_optional is None else dict(raw_optional)
    litellm_params: dict[str, JsonValue] = {} if raw_params is None else dict(raw_params)
    handler = RecordingHandler()
    _answered: object = CONFIG.dispatch_text_to_speech(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L62-L134
        model=call["model"], input=call["input"], aspeech=call.get("aspeech") is True,
        optional_params=optional_params, litellm_params_dict=litellm_params,
        logging_obj=SpeechLog(), timeout=float(call["timeout_seconds"]),
        api_base=call.get("api_base"), api_key=call.get("api_key"),
        voice=cast("str | dict[str, JsonValue] | None", call.get("voice")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L110-L118
        extra_headers=cast("dict[str, object] | None", call.get("extra_headers")),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L76
        base_llm_http_handler=cast("BaseLLMHTTPHandler", handler),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L119-L134
    )
    if not handler.calls:
        raise ValueError("the dispatch reaches the handler")
    return handler.calls[0]


class NoHeadersReply:
    def create(self, **kwargs: JsonValue) -> None:
        self.sent = kwargs


class NoHeadersClient:
    def __init__(self, **kwargs: JsonValue) -> None:
        self.options = kwargs
        self.chat = self
        self.completions = self
        self.with_raw_response = NoHeadersReply()


def probe_headers(
    scenario: ProbeScenario,
) -> tuple[dict[str, JsonValue] | None, dict[str, JsonValue] | None]:
    mode = scenario.get("mode")
    invalid = scenario["outcome"] == "invalid_completion"
    saved: type[AzureOpenAI] | None = None
    if invalid:
        saved = azure_module.AzureOpenAI  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1494-L1508
        azure_module.AzureOpenAI = NoHeadersClient  # type: ignore[attr-defined, assignment]  # pyright: ignore[reportPrivateImportUsage, reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1494-L1508
    headers: dict[str, JsonValue] | None = None
    raised: dict[str, JsonValue] | None = None
    try:
        raw_headers: dict[str, JsonValue] = AzureChatCompletion().get_headers(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1473-L1540
            model=scenario.get("model"), api_key=scenario["api_key"],
            api_base=scenario["api_base"], api_version=scenario["api_version"],
            timeout=600.0, mode="chat" if mode is None else mode,
            messages=scenario.get("messages"),
        )
        headers = OBJECT.validate_python(raw_headers)
    except openai.APIStatusError as caught:
        raised = {"status": caught.status_code}
    except Exception as caught:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1519-L1529
        raised = {"class": type(caught).__name__, "message": str(caught)}
    finally:
        if saved is not None:
            azure_module.AzureOpenAI = saved  # type: ignore[attr-defined]  # pyright: ignore[reportPrivateImportUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/azure.py#L1494-L1508
    return headers, raised


def probe_view(scenario: ProbeScenario) -> JsonValue:
    clear_ambient()
    litellm.in_memory_llm_clients_cache = LLMClientCache()
    reply = scenario.get("reply")
    recorder = Recorder({"status": 200, "body": "{}"} if reply is None else reply)
    litellm.client_session = httpx.Client(transport=httpx.MockTransport(recorder.respond))
    headers, raised = probe_headers(scenario)
    sent = recorder.seen[0] if recorder.seen else None
    request = None if sent is None else wire_view(sent)
    if scenario["outcome"] != "reply":
        return {"request": request, "reply": None, "error": raised}
    return {
        "request": request,
        "reply": None if headers is None else {"headers": headers},
        "error": raised,
    }


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    match next(iter(input_value), None):
        case "audio_speech":
            return audio_view(AUDIO.validate_python(input_value.get("audio_speech")))
        case "supported_params":
            supported_case = AUDIO.validate_python(input_value.get("supported_params"))
            supported: list[str] = CONFIG.get_supported_openai_params(supported_case["model"])  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L136-L143
            speech = audio_view(supported_case)
            return {"supported": JSON.validate_python(supported), "speech": speech}
        case "native_speech_selection":
            selection = OBJECT.validate_python(input_value.get("native_speech_selection"))
            calls = TypeAdapter(list[NativeCall]).validate_python(selection["calls"])
            return [native_view(call) for call in calls]
        case "probe":
            cases = OBJECT.validate_python(input_value.get("probe"))
            scenarios = TypeAdapter(list[ProbeScenario]).validate_python(cases["scenarios"])
            return [probe_view(scenario) for scenario in scenarios]
        case _:
            raise ValueError("expected an azure_speech variant")
