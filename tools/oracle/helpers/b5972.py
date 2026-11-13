import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.litellm_core_utils.prompt_templates import huggingface_template_handler
from litellm.llms.watsonx.chat.transformation import IBMWatsonXChatConfig
from litellm.llms.watsonx.common_utils import (
    WatsonXAIError,
    convert_watsonx_messages_to_prompt,  # pyright: ignore[reportUnknownVariableType]
)
from litellm.llms.watsonx.completion import transformation
from litellm.llms.watsonx.completion.transformation import (
    IBMWatsonXAIConfig,
    WatsonxTextCompletionResponseIterator,
)
from litellm.types.utils import ModelResponse

Json: TypeAlias = bool | int | float | str | list["Json"] | dict[str, "Json"] | None
Object: TypeAlias = dict[str, Json]
VARIABLES = """
WATSONX_IAM_URL WX_API_KEY WATSONX_API_KEY WATSONX_APIKEY WATSONX_ZENAPIKEY
WATSONX_TOKEN WATSONX_API_BASE WATSONX_URL WX_URL WML_URL
WATSONX_PROJECT_ID WX_PROJECT_ID PROJECT_ID WATSONX_REGION WX_REGION REGION
WATSONX_DEPLOYMENT_SPACE_ID WATSONX_SPACE_ID WX_SPACE_ID SPACE_ID
""".split()  # noqa: SIM905
FIXED_NOW = 1_700_000_000.0

CONFIG = IBMWatsonXAIConfig()
APPLY = cast("Callable[..., str | None]", IBMWatsonXChatConfig.apply_prompt_template)
CONVERT = cast("Callable[..., str | None]", convert_watsonx_messages_to_prompt)
SUPPORTED = cast("Callable[..., list[str]]", CONFIG.get_supported_openai_params)
TEXT_PARAM = cast("Callable[[str], bool]", CONFIG.is_watsonx_text_param)
MAPPED_AUTH = cast("Callable[[], dict[str, str]]", CONFIG.get_mapped_special_auth_params)
MAP_PARAMS = cast("Callable[..., Object]", CONFIG.map_openai_params)
MAP_AUTH = cast("Callable[..., Object]", CONFIG.map_special_auth_params)
TRANSFORM_REQUEST = cast("Callable[..., Object]", CONFIG.transform_request)
COMPLETE_URL = cast("Callable[..., str]", CONFIG.get_complete_url)
TRANSFORM_RESPONSE = cast("Callable[..., object]", CONFIG.transform_response)
EU_REGIONS = CONFIG.get_eu_regions
US_REGIONS = CONFIG.get_us_regions


class ChoiceMessage(Protocol):
    content: str | None


class Choice(Protocol):
    message: ChoiceMessage
    finish_reason: str


class Usage(Protocol):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class Completion(Protocol):
    choices: list[Choice]
    created: int
    usage: Usage


class Logging:
    def post_call(self, **params: Json) -> None:
        del params


def expect_text(value: Json) -> str:
    if not isinstance(value, str):
        raise TypeError("expected text")
    return value


def expect_text_or_none(value: Json) -> str | None:
    return None if value is None else expect_text(value)


def expect_list(value: Json) -> list[Json]:
    if not isinstance(value, list):
        raise TypeError("expected a list")
    return value


def expect_object(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def expect_bool(value: Json) -> bool:
    if not isinstance(value, bool):
        raise TypeError("expected a boolean")
    return value


def expect_number_or_none(value: Json) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError("expected a number")
    return float(value)


def _unreachable(hf_model_name: str) -> dict[str, Json]:
    del hf_model_name
    return {"status": "failure"}


@contextmanager
def patched_world(templates: Object, env: Object) -> Generator[None]:
    saved = {name: value for name, value in os.environ.items() if name in VARIABLES}
    known = cast("dict[str, Json]", litellm.known_tokenizer_config)  # pyright: ignore[reportPrivateImportUsage]
    saved_known = dict(known)
    for name in VARIABLES:
        os.environ.pop(name, None)
    try:
        for name, value in env.items():
            os.environ[name] = expect_text(value)
        for model, template in templates.items():
            known[model] = {
                "status": "success",
                "tokenizer": {
                    "chat_template": expect_text(template),
                    "bos_token": None,
                    "eos_token": None,
                },
            }
        with (
            patch.object(huggingface_template_handler, "_get_tokenizer_config", _unreachable),
            patch.object(huggingface_template_handler, "_get_chat_template_file", _unreachable),
        ):
            yield
    finally:
        for name in VARIABLES:
            os.environ.pop(name, None)
        os.environ.update(saved)
        known.clear()
        known.update(saved_known)


def status_of(error: Exception) -> Json:
    if isinstance(error, WatsonXAIError):
        return int(error.status_code)
    # an exception without a status code still ends as LiteLLM's internal server error
    return 500


def prompts(input_value: Object) -> Json:
    cases = [expect_object(entry) for entry in expect_list(input_value["cases"])]
    messages = cast("list[dict[str, Json]]", expect_list(input_value["messages"]))
    templates = expect_object(input_value.get("templates") or {})
    found: list[Json] = []
    with patched_world(templates, {}):
        for entry in cases:
            model = expect_text(entry["model"])
            provider = entry.get("provider")
            try:
                if provider is None:
                    found.append(APPLY(model=model, messages=messages))
                else:
                    found.append(
                        CONVERT(
                            model=model,
                            messages=messages,
                            provider=expect_text(provider),
                            custom_prompt_dict={},
                        )
                    )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                found.append({"ok": False, "status": status_of(error)})
    return {"prompts": found}


def map_params(input_value: Object) -> Json:
    cases = [expect_object(entry) for entry in expect_list(input_value["cases"])]
    probe = [expect_text(name) for name in expect_list(input_value["probe"])]
    results: list[Json] = []
    for entry in cases:
        non_default = expect_object(entry["non_default"])
        optional = expect_object(entry.get("optional") or {})
        options = MAP_PARAMS(
            non_default_params=dict(non_default),
            optional_params=dict(optional),
            model="model",
            drop_params=False,
        )
        auth = MAP_AUTH(non_default_params=dict(non_default), optional_params=dict(optional))
        results.append({"options": options, "auth": auth})
    return cast(
        "Json",
        {
            "results": results,
            "supported": SUPPORTED(model="model"),
            "text_param": [TEXT_PARAM(name) for name in probe],
            "special_auth": MAPPED_AUTH(),
            "eu_regions": EU_REGIONS(),
            "us_regions": US_REGIONS(),
        },
    )


def payload(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    templates = expect_object(input_value.get("templates") or {})
    env = expect_object(input_value.get("env") or {})
    bodies: list[Json] = []
    with patched_world(templates, env):
        for entry in requests:
            options = dict(expect_object(entry.get("options") or {}))
            try:
                body = TRANSFORM_REQUEST(
                    model=expect_text(entry["model"]),
                    messages=cast("list[dict[str, Json]]", expect_list(entry["messages"])),
                    optional_params=options,
                    litellm_params={},
                    headers={},
                )
                bodies.append({"ok": True, "body": body})
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                bodies.append({"ok": False, "status": status_of(error)})
    return {"bodies": bodies}


def url(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["requests"])]
    env = expect_object(input_value.get("env") or {})
    urls: list[Json] = []
    with patched_world({}, env):
        for entry in requests:
            options = dict(expect_object(entry.get("options") or {}))
            try:
                found = COMPLETE_URL(
                    api_base=expect_text_or_none(entry.get("api_base")),
                    api_key=None,
                    model=expect_text(entry["model"]),
                    optional_params=options,
                    litellm_params={},
                    stream=expect_bool(entry["stream"]),
                )
                urls.append({"ok": True, "url": found, "options": options})
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                urls.append({"ok": False, "status": status_of(error)})
    return {"urls": urls}


def reply(input_value: Object) -> Json:
    requests = [expect_object(entry) for entry in expect_list(input_value["replies"])]
    replies: list[Json] = []
    for entry in requests:
        raw = httpx.Response(
            200,
            content=expect_text(entry["body"]).encode(),
            request=httpx.Request("POST", "https://host"),
        )
        seconds = expect_number_or_none(entry.get("now"))
        clock = FIXED_NOW if seconds is None else seconds
        with patch.object(transformation.time, "time", return_value=clock):  # pyright: ignore[reportPrivateImportUsage]
            try:
                result = cast(
                    "Completion",
                    TRANSFORM_RESPONSE(
                        model="model",
                        raw_response=raw,
                        model_response=ModelResponse(),
                        logging_obj=Logging(),
                        request_data={},
                        messages=[],
                        optional_params={},
                        litellm_params={},
                        encoding=None,
                    ),
                )
                choice = result.choices[0]
                replies.append(
                    {
                        "ok": True,
                        "text": choice.message.content,
                        "finish_reason": choice.finish_reason,
                        "created": result.created,
                        "usage": {
                            "prompt_tokens": result.usage.prompt_tokens,
                            "completion_tokens": result.usage.completion_tokens,
                            "total_tokens": result.usage.total_tokens,
                        },
                    }
                )
            except Exception as error:  # noqa: BLE001 - the wire contract carries every helper exception
                replies.append({"ok": False, "status": status_of(error)})
    return {"replies": replies}


def stream_line(input_value: Object) -> Json:
    lines = [expect_text(line) for line in expect_list(input_value["lines"])]
    iterator = WatsonxTextCompletionResponseIterator(
        streaming_response=iter([]),
        sync_stream=True,
        json_mode=False,
    )
    handle = cast(
        "Callable[[str], dict[str, Json]]",
        iterator._handle_string_chunk,  # pyright: ignore[reportPrivateUsage]  # noqa: SLF001
    )
    chunks: list[Json] = []
    for line in lines:
        chunk = handle(line)
        chunks.append(
            {
                "text": chunk.get("text"),
                "is_finished": chunk.get("is_finished"),
                "finish_reason": chunk.get("finish_reason"),
                "usage": chunk.get("usage"),
            }
        )
    return {"chunks": chunks}


OPS: dict[str, Callable[[Object], Json]] = {
    "prompts": prompts,
    "map_params": map_params,
    "payload": payload,
    "url": url,
    "reply": reply,
    "stream_line": stream_line,
}


def run(input_value: Object) -> Json:
    operation = expect_text(input_value["op"])
    return OPS[operation](input_value)
