import json
import os
from collections.abc import Callable, Generator
from contextlib import contextmanager
from datetime import UTC, datetime, timedelta
from functools import partial
from pathlib import Path
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.base_llm.embedding.transformation import LiteLLMLoggingObj
from litellm.llms.sap import credentials as sap_credentials
from litellm.llms.sap.chat import transformation as sap_chat
from litellm.llms.sap.embed import transformation as sap_embed
from litellm.types.utils import EmbeddingResponse, ModelResponse

if TYPE_CHECKING:
    from litellm.types.llms.openai import AllEmbeddingInputValues, AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

AUTHORIZATION = "Bearer case-token"
BASE_URL = "https://base.example.com/v2"
RESOURCE_GROUP = "default"
DEFAULT_HOME = "/nonexistent-home"


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def optional_string(value: Json) -> str | None:
    if value is None:
        return None
    return string_value(value, "string")


def session_field(call: dict[str, Json], name: str, default: str = "") -> str:
    session = object_value(call.get("session") or {}, "session")
    found = optional_string(session.get(name))
    return default if found is None else found


@contextmanager
def patched_environment(call: dict[str, Json]) -> Generator[None, None, None]:
    environment = object_value(call.get("environment") or {}, "environment")
    variables = {
        name: string_value(value, name)
        for name, value in object_value(environment.get("vars") or {}, "vars").items()
    }
    with (
        patch.dict(os.environ, variables, clear=True),
        patch.object(sap_credentials, "DEFAULT_HOME_PATH", str(Path(DEFAULT_HOME) / ".aicore")),
        patch.object(Path, "exists", return_value=False),
        patch.object(litellm, "sap_service_key", None),
    ):
        yield


def fixed_token_creator(
    token: str,
    base_url: str,
    resource_group: str,
    *_args: object,
    **_kwargs: object,
) -> tuple[Callable[[], str], str, str]:
    return (lambda: token), base_url, resource_group


def fixed_token_request(**_kwargs: object) -> tuple[str, datetime]:
    return AUTHORIZATION, datetime.now(UTC) + timedelta(hours=2)


@contextmanager
def patched_token_request() -> Generator[None, None, None]:
    with patch.object(sap_credentials, "_request_token", fixed_token_request):
        yield


def header_pairs(headers: dict[str, str]) -> Json:
    return cast(Json, sorted([name.lower(), value] for name, value in headers.items()))


def chat_environment(call: dict[str, Json]) -> Json:
    service_key = optional_string(call.get("service_key"))
    with patched_environment(call), patched_token_request():
        config = sap_chat.GenAIHubOrchestrationConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
        config.__dict__["deployment_url"] = session_field(call, "deployment_url")  # pyright: ignore[reportUnknownMemberType,reportIndexIssue]
        headers = cast(
            dict[str, str],
            config.validate_environment({}, "m", [], {}, {}, api_key=service_key),  # pyright: ignore[reportUnknownMemberType]
        )
        url = config.get_complete_url(None, None, "m", {}, {}, None)  # pyright: ignore[reportUnknownMemberType]
    return {"headers": header_pairs(headers), "url": url}


def chat_request(call: dict[str, Json]) -> Json:
    config = sap_chat.GenAIHubOrchestrationConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
    messages = array_value(call.get("messages") or [], "messages")
    params = object_value(call.get("optional_params") or {}, "optional_params")
    return cast(
        Json,
        config.transform_request(  # pyright: ignore[reportUnknownMemberType]
            model=string_value(call.get("model"), "model"),
            messages=cast("list[AllMessageValues]", messages),
            optional_params=dict(params),
            litellm_params={},
            headers={},
        ),
    )


class FakeResponse:
    def __init__(self, text: str) -> None:
        self.text = text

    def json(self) -> Json:
        return cast(Json, json.loads(self.text))


class Recorder:
    def __init__(self) -> None:
        self.calls: list[Json] = []

    def post_call(self, **fields: Json) -> None:
        complete = object_value(fields.get("additional_args") or {}, "additional_args")
        self.calls.append(
            {
                "input": fields.get("input"),
                "api_key": fields.get("api_key"),
                "original_response": fields.get("original_response"),
                "complete_input_dict": complete.get("complete_input_dict"),
            }
        )


def chat_response_view(response: ModelResponse) -> Json:
    dumped = cast(dict[str, Json], response.model_dump(mode="json", exclude_none=True))  # pyright: ignore[reportUnknownMemberType]
    choices: list[Json] = []
    for item in array_value(dumped.get("choices") or [], "choices"):
        fields = object_value(item, "choice")
        message = object_value(fields.get("message") or {}, "message")
        choices.append(
            {
                "index": fields.get("index"),
                "finish_reason": fields.get("finish_reason"),
                "message": {"role": message.get("role"), "content": message.get("content")},
            }
        )
    usage = dumped.get("usage")
    return {
        "id": dumped.get("id"),
        "object": dumped.get("object"),
        "created": dumped.get("created"),
        "model": dumped.get("model"),
        "choices": choices,
        "usage": None
        if usage is None
        else {
            name: object_value(usage, "usage").get(name)
            for name in ("prompt_tokens", "completion_tokens", "total_tokens")
        },
    }


def chat_response(call: dict[str, Json]) -> Json:
    config = sap_chat.GenAIHubOrchestrationConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
    recorder = Recorder()
    optional_params: dict[str, Json] = {}
    if call.get("response_format") is not None:
        optional_params["response_format"] = call.get("response_format")
    response = config.transform_response(  # pyright: ignore[reportUnknownMemberType]
        model=string_value(call.get("model"), "model"),
        raw_response=cast(
            httpx.Response,
            FakeResponse(string_value(call.get("raw_text"), "raw_text")),
        ),
        model_response=ModelResponse(),
        logging_obj=cast("sap_chat.LiteLLMLoggingObj", recorder),
        request_data=cast(dict[str, Json], call.get("request_data") or {}),
        messages=cast(
            "list[AllMessageValues]",
            array_value(call.get("messages") or [], "messages"),
        ),
        optional_params=optional_params,
        litellm_params={},
        encoding=None,
        api_key=optional_string(call.get("api_key")),
        json_mode=None,
    )
    return {"response": chat_response_view(response), "post_call": recorder.calls}


def embedding_config(call: dict[str, Json]) -> sap_embed.GenAIHubEmbeddingConfig:
    creator = partial(
        fixed_token_creator,
        session_field(call, "token", AUTHORIZATION),
        session_field(call, "base_url", BASE_URL),
        session_field(call, "resource_group", RESOURCE_GROUP),
    )
    with patch.object(sap_embed, "get_token_creator", creator):
        config = sap_embed.GenAIHubEmbeddingConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
    config.__dict__["deployment_url"] = session_field(call, "deployment_url")  # pyright: ignore[reportUnknownMemberType,reportIndexIssue]
    return config


def embedding_call(call: dict[str, Json]) -> Json:
    function = string_value(call.get("function"), "function")
    config = embedding_config(call)
    if function == "prepare":
        headers = cast(dict[str, str], config.validate_environment({}))  # pyright: ignore[reportUnknownMemberType]
        url = config.get_complete_url(None, None, "m", {}, {}, None)  # pyright: ignore[reportUnknownMemberType]
        return {"headers": header_pairs(headers), "url": url}
    if function == "map_params":
        return cast(
            Json,
            config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]
                cast(dict[str, Json], call.get("non_default_params") or {}),
                cast(dict[str, Json], call.get("optional_params") or {}),
                string_value(call.get("model"), "model"),
                bool(call.get("drop_params")),
            ),
        )
    if function == "response":
        response = config.transform_embedding_response(  # pyright: ignore[reportUnknownMemberType]
            model="m",
            raw_response=cast(
                httpx.Response,
                FakeResponse(string_value(call.get("raw_text"), "raw_text")),
            ),
            model_response=cast(EmbeddingResponse, None),
            logging_obj=cast(LiteLLMLoggingObj, None),
            api_key=None,
            request_data={},
            optional_params={},
            litellm_params={},
        )
        return cast(Json, response.model_dump(mode="json"))  # pyright: ignore[reportUnknownMemberType]
    if function == "error_class":
        error = config.get_error_class(  # pyright: ignore[reportUnknownMemberType]
            string_value(call.get("message"), "message"),
            cast(int, call.get("status")),
            {},
        )
        return {"status": error.status_code, "message": error.message}
    return cast(
        Json,
        config.transform_embedding_request(  # pyright: ignore[reportUnknownMemberType]
            string_value(call.get("model"), "model"),
            cast("AllEmbeddingInputValues", call.get("input")),
            cast(dict[str, Json], call.get("optional_params") or {}),
            {},
        ),
    )


def supported_params(call: dict[str, Json]) -> Json:
    model = string_value(call.get("model"), "model")
    if optional_string(call.get("family")) == "embedding":
        config = embedding_config(call)
        return list(cast(list[Json], config.get_supported_openai_params(model)))  # pyright: ignore[reportUnknownMemberType]
    chat = sap_chat.GenAIHubOrchestrationConfig()  # pyright: ignore[reportUnknownMemberType,reportUnknownVariableType]
    return list(cast(list[Json], chat.get_supported_openai_params(model)))  # pyright: ignore[reportUnknownMemberType]


def run(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    calls = array_value(input_value.get("calls") or [], "calls")
    functions: dict[str, Callable[[dict[str, Json]], Json]] = {
        "supported_params": supported_params,
        "requests": chat_request,
        "responses": chat_response,
        "environments": chat_environment,
        "embeddings": embedding_call,
    }
    function = functions.get(operation)
    if function is None:
        raise ValueError(f"unknown operation {operation}")
    outcomes: list[Json] = []
    for call in calls:
        try:
            outcomes.append({"ok": True, "value": function(object_value(call, "call"))})
        except Exception as error:  # noqa: BLE001 - the helper reports whichever error the call raised
            carried: object = getattr(error, "status_code", None)
            status = carried if isinstance(carried, int) and not isinstance(carried, bool) else None
            outcomes.append({"ok": False, "status": status})
    return outcomes
