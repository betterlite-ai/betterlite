import os
import re
from datetime import UTC, datetime
from typing import Final, Literal, NamedTuple, Protocol, Required, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.text_to_speech.transformation import BaseTextToSpeechConfig
from litellm.types.llms.openai import HttpxBinaryResponseContent
from litellm.types.utils import LlmProviders
from litellm.utils import ProviderConfigManager
from pydantic import JsonValue, TypeAdapter

MODEL = "aws_polly/neural"
DEFAULT_TIMEOUT = 600.0
EXAMPLE_ACCESS_KEY = "AKIDEXAMPLE"
EXAMPLE_SIGNING_VALUE = "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY"

litellm.suppress_debug_info = True

Object: TypeAlias = dict[str, JsonValue]
OPTIONAL_TEXT: Final[TypeAdapter[str | None]] = TypeAdapter(str | None)
OBJECT: Final[TypeAdapter[Object]] = TypeAdapter(Object)


class SpeechRequestData(TypedDict, total=False):
    dict_body: Object
    ssml_body: str
    headers: dict[str, str]


REQUEST_DATA: Final[TypeAdapter[SpeechRequestData]] = TypeAdapter(SpeechRequestData)


class Signed(TypedDict):
    scheme: str
    credential: str
    scope: list[str]
    signed_headers: list[str]
    signature_ok: bool
    date_ok: bool
    header_names: list[str]
    content_type: str | None
    security_token: str | None


class Handler(Protocol):
    def text_to_speech_handler(self, **kwargs: JsonValue) -> HttpxBinaryResponseContent: ...


class Config(Protocol):
    def map_openai_params(
        self, model: str, optional_params: Object, voice: str | dict[str, JsonValue] | None,
        /, *, drop_params: bool, kwargs: Object
    ) -> tuple[str | None, Object]: ...

    def get_complete_url(
        self, model: str, api_base: str | None, litellm_params: Object, /) -> str: ...

    # ast-grep-ignore: no-long-names
    def transform_text_to_speech_request(  # noqa: PLR0913  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/aws_polly/text_to_speech/transformation.py#L310-L318
        self, model: str, text: str, voice: str | None, optional_params: Object,
        litellm_params: Object, headers: dict[str, str], /
    ) -> SpeechRequestData: ...

    def dispatch_text_to_speech(
        self, model: str, text: str, voice: str | dict[str, JsonValue] | None, optional_params: Object,
        litellm_params_dict: Object, logging_obj: Logging, timeout: float, extra_headers: None,
        base_llm_http_handler: Handler, aspeech: bool, api_base: str | None, api_key: str | None,
        /, **kwargs: JsonValue
    ) -> HttpxBinaryResponseContent: ...


BASE: BaseTextToSpeechConfig | None = ProviderConfigManager.get_provider_text_to_speech_config(
    model=MODEL, provider=LlmProviders.AWS_POLLY
)
if BASE is None:
    raise RuntimeError("LiteLLM has no AWS Polly text-to-speech config")
CONFIG: Final[Config] = cast(Config, BASE)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/aws_polly/text_to_speech/transformation.py#L30
LOG: Final = Logging(
    model="", messages=None, stream=False, call_type="", start_time=datetime.now(UTC),
    litellm_call_id="", function_id="",
)


class Call(TypedDict, total=False):
    function: Required[Literal["map_params", "url", "request", "dispatch", "credential_arguments"]]
    model: Required[str]
    text: Required[str]
    optional_params: Required[Object]
    litellm_params: Required[Object]
    kwargs: Required[Object]
    environment: Required[dict[str, str]]
    voice: str | dict[str, JsonValue] | None
    kwargs_region: str | None
    api_base: str | None
    api_key: str | None


class Input(TypedDict):
    calls: list[Call]


class Failure(TypedDict):
    ok: bool


class Mapped(TypedDict):
    ok: bool
    voice: str | None
    params: Object


class Located(TypedDict):
    ok: bool
    url: str


class Dispatched(TypedDict):
    ok: bool
    voice: str | None
    optional_params: Object
    litellm_params: Object


class Requested(TypedDict):
    ok: bool
    url: str
    dict_body_absent: bool
    body: str
    signed: Signed


class Arguments(TypedDict):
    ok: bool
    arguments: Object


Answer: TypeAlias = Mapped | Located | Dispatched | Requested | Arguments | Failure


class Output(TypedDict):
    calls: list[Answer]


class SigningCredentials(NamedTuple):
    access_key: str
    secret_key: str
    token: str | None


class RecordedCredentialsError(Exception): ...


class CredentialProbe:
    def __init__(self, *, sign: bool) -> None:
        self.calls: list[Object] = []
        self.sign = sign

    def get_credentials(self, **kwargs: JsonValue) -> SigningCredentials:
        self.calls.append(dict(kwargs))
        access_key = OPTIONAL_TEXT.validate_python(kwargs["aws_access_key_id"])
        secret_key = OPTIONAL_TEXT.validate_python(kwargs["aws_secret_access_key"])
        if not self.sign:
            raise RecordedCredentialsError
        token = OPTIONAL_TEXT.validate_python(kwargs["aws_session_token"])
        return SigningCredentials(
            EXAMPLE_ACCESS_KEY if access_key is None else access_key,
            EXAMPLE_SIGNING_VALUE if secret_key is None else secret_key,
            token,
        )


SIGNING: Final = CredentialProbe(sign=True)
RECORDING: Final = CredentialProbe(sign=False)


class Recorder:
    def __init__(self) -> None:
        self.calls: list[Object] = []

    def text_to_speech_handler(self, **kwargs: JsonValue) -> HttpxBinaryResponseContent:
        self.calls.append(dict(kwargs))
        return HttpxBinaryResponseContent(httpx.Response(200, content=b"handler-answer"))


def apply_environment(environment: dict[str, str]) -> None:
    for name in list(os.environ):
        if name.startswith("AWS_"):
            del os.environ[name]
    os.environ.update(environment)


def signed_view(headers: dict[str, str]) -> Signed:
    lowered = {name.lower(): value for name, value in headers.items()}
    scheme, _, rest = lowered["authorization"].partition(" ")
    fields = {name: value for name, _, value in (part.partition("=") for part in rest.split(", "))}
    parts = fields["Credential"].split("/")
    return {
        "scheme": scheme,
        "credential": parts[0],
        "scope": parts[2:5],
        "signed_headers": fields["SignedHeaders"].split(";"),
        "signature_ok": re.fullmatch("[0-9a-f]{64}", fields["Signature"]) is not None,
        "date_ok": re.fullmatch("[0-9]{8}T[0-9]{6}Z", lowered["x-amz-date"]) is not None,
        "header_names": sorted(lowered),
        "content_type": lowered.get("content-type"),
        "security_token": lowered.get("x-amz-security-token"),
    }


def map_answer(call: Call) -> Answer:
    voice, params = CONFIG.map_openai_params(
        call["model"], call["optional_params"], call.get("voice"), drop_params=False, kwargs=call["kwargs"]
    )
    return {"ok": True, "voice": voice, "params": params}


def url_answer(call: Call) -> Answer:
    url = CONFIG.get_complete_url(MODEL, call.get("api_base"), call["litellm_params"])
    return {"ok": True, "url": url}


def dispatch_answer(call: Call) -> Answer:
    apply_environment(call["environment"])
    object.__setattr__(BASE, "get_credentials", SIGNING.get_credentials)
    recorder = Recorder()
    try:
        CONFIG.dispatch_text_to_speech(
            MODEL, "", call.get("voice"), call["optional_params"], call["litellm_params"],
            LOG, DEFAULT_TIMEOUT, None, recorder, False, call.get("api_base"),
            call.get("api_key"), aws_region_name=call.get("kwargs_region"),
        )
    except ValueError:
        return {"ok": False}
    received = recorder.calls[-1]
    return {
        "ok": True,
        "voice": OPTIONAL_TEXT.validate_python(received["voice"]),
        "optional_params": OBJECT.validate_python(received["text_to_speech_optional_params"]),
        "litellm_params": OBJECT.validate_python(received["litellm_params"]),
    }


def request_answer(call: Call) -> Answer:
    apply_environment({})
    object.__setattr__(BASE, "get_credentials", SIGNING.get_credentials)
    try:
        data = REQUEST_DATA.validate_python(
            CONFIG.transform_text_to_speech_request(
                MODEL, call["text"], OPTIONAL_TEXT.validate_python(call.get("voice")),
                call["optional_params"], call["litellm_params"], {},
            )
        )
        api_base = OPTIONAL_TEXT.validate_python(call["litellm_params"].get("api_base"))
        url = CONFIG.get_complete_url(MODEL, api_base, call["litellm_params"])
    except ValueError:
        return {"ok": False}
    headers = data.get("headers")
    body = data.get("ssml_body")
    if headers is None or body is None:
        raise ValueError("the Polly transformation returns signed headers and a body")
    return {
        "ok": True,
        "url": url,
        "dict_body_absent": "dict_body" not in data,
        "body": body,
        "signed": signed_view(headers),
    }


def credential_answer(call: Call) -> Answer:
    apply_environment({})
    object.__setattr__(BASE, "get_credentials", RECORDING.get_credentials)
    RECORDING.calls.clear()
    try:
        CONFIG.transform_text_to_speech_request(MODEL, "x", None, {}, call["litellm_params"], {})
    except RecordedCredentialsError:
        return {"ok": True, "arguments": RECORDING.calls[-1]}
    except ValueError:
        return {"ok": False}
    raise ValueError("the Polly signing never asks for credentials")


def answer(call: Call) -> Answer:
    if call["function"] == "map_params":
        return map_answer(call)
    if call["function"] == "url":
        return url_answer(call)
    if call["function"] == "dispatch":
        return dispatch_answer(call)
    if call["function"] == "request":
        return request_answer(call)
    return credential_answer(call)


def run(input_value: Input) -> Output:
    answers: list[Answer] = [answer(call) for call in input_value["calls"]]
    return {"calls": answers}
