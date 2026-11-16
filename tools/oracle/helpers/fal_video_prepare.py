import os
import time
from collections.abc import Callable
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict, cast

import httpx
from litellm.llms.fal_ai.videos.transformation import FalAIVideoConfig
from litellm.types.router import GenericLiteLLMParams
from litellm.types.videos.main import VideoCreateOptionalRequestParams, VideoObject
from litellm.types.videos.utils import encode_video_id_with_provider
from pydantic import JsonValue, TypeAdapter

PROVIDER: Final = "fal_ai"
ENVIRONMENT_NAMES: Final[tuple[str, ...]] = (
    "FAL_AI_API_KEY",
    "FAL_AI_API_BASE",
    "FAL_AI_QUEUE_API_BASE",
)
PARAMS: TypeAdapter[dict[str, JsonValue]] = TypeAdapter(dict[str, JsonValue])
REJECTED: Final[JsonValue] = {"error": {"status": None}}


class ErrorCase(TypedDict):
    message: str
    status: int
    headers: dict[str, str]


class CreateCase(TypedDict):
    body: str
    request_data: dict[str, JsonValue]


class KeyCase(TypedDict):
    api_key: str | None
    litellm_api_key: str | None
    headers: dict[str, str]


class JobCase(TypedDict):
    model: str
    request_id: str


class H3AutoDuration(TypedDict):
    operation: Literal["h3_auto_duration"]
    models: list[str]
    seconds: list[JsonValue]


class SeedanceDecimalDuration(TypedDict):
    operation: Literal["seedance_decimal_duration"]
    models: list[str]
    seconds: list[JsonValue]


class ResolutionShortSide(TypedDict):
    operation: Literal["resolution_short_side"]
    models: list[str]
    sizes: list[JsonValue]


class ReferenceType(TypedDict):
    operation: Literal["reference_type"]
    models: list[str]
    references: list[JsonValue]


class ProviderOverride(TypedDict):
    operation: Literal["provider_override"]
    model: str
    now: int
    providers: list[str | None]
    cases: list[CreateCase]
    errors: list[ErrorCase]


class QueueBaseLiteral(TypedDict):
    operation: Literal["queue_base_literal"]
    environments: list[dict[str, str]]
    api_bases: list[str | None]
    cases: list[KeyCase]


class WorkflowJobPath(TypedDict):
    operation: Literal["workflow_job_path"]
    api_bases: list[str | None]
    jobs: list[JobCase]


Input: TypeAlias = (
    H3AutoDuration
    | SeedanceDecimalDuration
    | ResolutionShortSide
    | ReferenceType
    | ProviderOverride
    | QueueBaseLiteral
    | WorkflowJobPath
)


class VideoFields(TypedDict):
    id: str
    object: str
    status: str
    created_at: int | None
    completed_at: int | None
    expires_at: int | None
    error: dict[str, JsonValue] | None
    progress: int | None
    remixed_from_video_id: str | None
    seconds: str | None
    size: str | None
    model: str | None
    usage: dict[str, JsonValue] | None


VIDEO_FIELDS: TypeAdapter[VideoFields] = TypeAdapter(VideoFields)


def apply_environment(environment: dict[str, str]) -> None:
    for name in ENVIRONMENT_NAMES:
        os.environ.pop(name, None)
    for name, value in environment.items():
        os.environ[name] = value


def outcome(call: Callable[[], JsonValue]) -> JsonValue:
    try:
        return call()
    except ValueError:
        return REJECTED


def mapped_params(
    config: FalAIVideoConfig, key: str, value: JsonValue, model: str
) -> JsonValue:
    params: dict[str, JsonValue] = {key: value}
    request = cast("VideoCreateOptionalRequestParams", params)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/videos/transformation.py#L301
    mapped = config.map_openai_params(request, model, drop_params=False)
    return {"params": PARAMS.validate_python(mapped)}


def mapping_report(
    config: FalAIVideoConfig, models: list[str], key: str, values: list[JsonValue]
) -> JsonValue:
    results: list[JsonValue] = []
    for model in models:
        cases = [
            outcome(partial(mapped_params, config, key, value, model))
            for value in values
        ]
        results.append({"model": model, "cases": cases})
    return {"results": results}


def video_fields(video: VideoObject) -> JsonValue:
    fields = VIDEO_FIELDS.validate_json(video.model_dump_json())
    return {
        "id": fields["id"],
        "object": fields["object"],
        "status": fields["status"],
        "created_at": fields["created_at"],
        "completed_at": fields["completed_at"],
        "expires_at": fields["expires_at"],
        "error": fields["error"],
        "progress": fields["progress"],
        "remixed_from_video_id": fields["remixed_from_video_id"],
        "seconds": fields["seconds"],
        "size": fields["size"],
        "model": fields["model"],
        "usage": fields["usage"],
    }


def created_video(
    config: FalAIVideoConfig, model: str, provider: str | None, case: CreateCase
) -> JsonValue:
    raw = httpx.Response(
        status_code=200,
        content=case["body"].encode(),
        request=httpx.Request("POST", f"https://queue.fal.run/{model}"),
    )
    video = config.transform_video_create_response(
        model,
        raw,
        None,
        custom_llm_provider=provider,
        request_data=case["request_data"],
    )
    return video_fields(video)


def error_fields(config: FalAIVideoConfig, case: ErrorCase) -> JsonValue:
    error = config.get_error_class(
        case["message"], case["status"], dict(case["headers"])
    )
    return {"status_code": error.status_code, "message": error.message}


def provider_report(config: FalAIVideoConfig, call: ProviderOverride) -> JsonValue:
    time.time = partial(float, call["now"])
    return {
        "videos": [
            outcome(partial(created_video, config, call["model"], provider, case))
            for provider in call["providers"]
            for case in call["cases"]
        ],
        "errors": [error_fields(config, case) for case in call["errors"]],
    }


def key_headers(config: FalAIVideoConfig, case: KeyCase) -> JsonValue:
    headers = config.validate_environment(
        dict(case["headers"]),
        "",
        api_key=case["api_key"],
        litellm_params=GenericLiteLLMParams(api_key=case["litellm_api_key"]),
    )
    return {"headers": PARAMS.validate_python(headers)}


def environment_report(
    config: FalAIVideoConfig, environment: dict[str, str], call: QueueBaseLiteral
) -> JsonValue:
    apply_environment(environment)
    urls: list[JsonValue] = [
        config.get_complete_url("", base, {}) for base in call["api_bases"]
    ]
    headers: list[JsonValue] = [
        outcome(partial(key_headers, config, case)) for case in call["cases"]
    ]
    return {"urls": urls, "headers": headers}


def job_url(
    config: FalAIVideoConfig,
    kind: Literal["status", "content"],
    video_id: str,
    api_base: str,
) -> JsonValue:
    params = GenericLiteLLMParams()
    if kind == "status":
        url = config.transform_video_status_retrieve_request(
            video_id, api_base, params, {}
        )[0]
    else:
        url = config.transform_video_content_request(video_id, api_base, params, {})[0]
    return {"url": url}


def job_report(config: FalAIVideoConfig, call: WorkflowJobPath) -> JsonValue:
    jobs: list[JsonValue] = []
    for api_base in call["api_bases"]:
        base = config.get_complete_url("", api_base, {})
        for job in call["jobs"]:
            video_id = encode_video_id_with_provider(
                job["request_id"], PROVIDER, job["model"]
            )
            status = outcome(partial(job_url, config, "status", video_id, base))
            content = outcome(partial(job_url, config, "content", video_id, base))
            jobs.append({"status": status, "content": content})
    return {"jobs": jobs}


def run(input_value: Input) -> JsonValue:
    config = FalAIVideoConfig()
    match input_value["operation"]:
        case "h3_auto_duration" | "seedance_decimal_duration":
            return mapping_report(
                config, input_value["models"], "seconds", input_value["seconds"]
            )
        case "resolution_short_side":
            return mapping_report(
                config, input_value["models"], "size", input_value["sizes"]
            )
        case "reference_type":
            return mapping_report(
                config,
                input_value["models"],
                "input_reference",
                input_value["references"],
            )
        case "provider_override":
            return provider_report(config, input_value)
        case "queue_base_literal":
            return {
                "environments": [
                    environment_report(config, environment, input_value)
                    for environment in input_value["environments"]
                ]
            }
        case "workflow_job_path":
            return job_report(config, input_value)
