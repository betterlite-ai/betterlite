import builtins
import os
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

import httpx
from litellm.llms.infinity.embedding.transformation import InfinityEmbeddingConfig
from litellm.llms.jina_ai.embedding.transformation import JinaAIEmbeddingConfig
from litellm.types.utils import EmbeddingResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Input = str | list[str] | list[list[str]] | list[int] | list[list[int]]


class Recorder:
    def __init__(self) -> None:
        self.calls: list[dict[str, object]] = []

    def post_call(self, **kwargs: object) -> None:
        self.calls.append(kwargs)


class Raised(Protocol):
    status_code: int
    message: str
    headers: dict[str, str]
    request: httpx.Request


class HostConfig(Protocol):
    def validate_environment(
        self, headers: dict[str, str], model: str, messages: list[Json],
        optional_params: dict[str, Json], litellm_params: dict[str, Json],
        **credentials: str | None
    ) -> dict[str, str]: ...

    def map_openai_params(
        self, non_default_params: dict[str, Json], optional_params: dict[str, Json],
        model: str, *, drop_params: bool
    ) -> dict[str, Json]: ...

    def transform_embedding_request(
        self, model: str, input_data: Input, optional_params: dict[str, Json],
        headers: dict[str, Json]
    ) -> dict[str, Json]: ...

    def get_complete_url(
        self, api_base: str | None, api_key: str | None, model: str,
        optional_params: dict[str, Json], litellm_params: dict[str, Json]
    ) -> str: ...

    def transform_embedding_response(
        self, model: str, raw_response: httpx.Response, model_response: EmbeddingResponse,
        logging_obj: Recorder, **context: str | dict[str, Json] | None
    ) -> EmbeddingResponse: ...

    def get_error_class(
        self, error_message: str, status_code: int, headers: dict[str, str]
    ) -> Raised: ...


class JinaConfig(HostConfig, Protocol):
    def _get_openai_compatible_provider_info(
        self, api_base: str | None, api_key: str | None
    ) -> tuple[str, str, str | None]: ...


def extras(call: dict[str, object]) -> dict[str, object]:
    return cast(dict[str, object], call["additional_args"])


def project(response: EmbeddingResponse) -> dict[str, Json]:
    fields = cast(dict[str, Json], response.model_dump())
    usage = cast(dict[str, Json], fields["usage"])
    return {
        "model": fields["model"], "object": fields["object"], "data": fields["data"],
        "usage": {"prompt_tokens": usage["prompt_tokens"], "total_tokens": usage["total_tokens"]},
    }


def decode(
    config: HostConfig, value: dict[str, Json], request: dict[str, Json], api_key: str | None,
    recorder: Recorder
) -> dict[str, Json] | None:
    reply = cast(dict[str, Json] | None, value.get("reply"))
    if reply is None:
        return None
    raw = httpx.Response(cast(int, reply["status"]), content=cast(str, reply["body"]))
    response = config.transform_embedding_response(
        cast(str, value["model"]), raw, EmbeddingResponse(), recorder,
        api_key=api_key, request_data=request, optional_params={}, litellm_params={},
    )
    return project(response)


def mapped(config: HostConfig, value: dict[str, Json]) -> dict[str, Json]:
    options = cast(dict[str, Json], value.get("options", {}))
    seeded = dict(cast(dict[str, Json], value.get("optional", {})))
    return config.map_openai_params(options, seeded, cast(str, value["model"]), drop_params=False)


def jina(value: dict[str, Json]) -> dict[str, Json]:
    config = cast(JinaConfig, JinaAIEmbeddingConfig())
    model = cast(str, value["model"])
    base = cast(str | None, value.get("base"))
    with patch.dict(os.environ, cast(dict[str, str], value.get("env", {})), clear=True):
        _, api_base, api_key = config._get_openai_compatible_provider_info(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            base, cast(str | None, value.get("api_key"))
        )
    request = config.transform_embedding_request(
        model, cast(Input, value["input"]), mapped(config, value), {}
    )
    recorder = Recorder()
    response = decode(config, value, request, api_key, recorder)
    hook: list[Json] = [
        {
            "input_is_builtin": call["input"] is builtins.input,
            "api_key": cast(str | None, call["api_key"]),
            "complete_input_dict": cast(dict[str, Json], extras(call)["complete_input_dict"]),
            "original_response": cast(Json, call["original_response"]),
        }
        for call in recorder.calls
    ]
    return {
        "api_base": api_base, "api_key": api_key,
        "url": config.get_complete_url(base, None, model, {}, {}),
        "headers": cast(
            dict[str, Json],
            config.validate_environment(
                dict(cast(dict[str, str], value.get("headers", {}))), model, [], {}, {},
                api_key=api_key,
            ),
        ),
        "request": request, "response": response, "hook": hook,
    }


def infinity(value: dict[str, Json]) -> dict[str, Json]:
    config = cast(HostConfig, InfinityEmbeddingConfig())
    model = cast(str, value["model"])
    with patch.dict(os.environ, cast(dict[str, str], value.get("env", {})), clear=True):
        headers = config.validate_environment(
            dict(cast(dict[str, str], value.get("headers", {}))), model, [], {}, {},
            api_key=cast(str | None, value.get("api_key")),
        )
    failure = cast(dict[str, Json] | None, value.get("error"))
    error: dict[str, Json] | None = None
    if failure is not None:
        raised = config.get_error_class(
            cast(str, failure["message"]),
            cast(int, failure["status"]),
            cast(dict[str, str], failure.get("headers", {})),
        )
        error = {
            "status_code": raised.status_code, "message": raised.message,
            "headers": cast(Json, raised.headers), "url": str(raised.request.url),
        }
    return {
        "url": config.get_complete_url(cast(str | None, value["base"]), None, model, {}, {}),
        "headers": cast(dict[str, Json], headers),
        "request": config.transform_embedding_request(
            model, cast(Input, value["input"]), mapped(config, value), {}
        ),
        "response": decode(config, value, {}, None, Recorder()), "error": error,
    }


def run(value: dict[str, Json]) -> Json:
    return {
        "jina": jina(cast(dict[str, Json], value["jina"])),
        "infinity": infinity(cast(dict[str, Json], value["infinity"])),
    }
