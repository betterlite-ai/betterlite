import os
import time
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


MappingCall: TypeAlias = (
    H3AutoDuration | SeedanceDecimalDuration | ResolutionShortSide | ReferenceType
)
Input: TypeAlias = (
    MappingCall | ProviderOverride | QueueBaseLiteral | WorkflowJobPath
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


def mapping_outcome(
    config: FalAIVideoConfig, params: dict[str, JsonValue], model: str
) -> JsonValue:
    try:
        mapped = config.map_openai_params(
            cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fal_ai/videos/transformation.py#L301
                "VideoCreateOptionalRequestParams",
                params,
            ),
            model,
            drop_params=False,
        )
    except ValueError:
        return {"error": {"status": None}}
    return {"params": PARAMS.validate_python(mapped)}


def mapping_report(
    config: FalAIVideoConfig, models: list[str], key: str, values: list[JsonValue]
) -> JsonValue:
    return {
        "results": [
            {
                "model": model,
                "cases": [
                    mapping_outcome(config, {key: value}, model) for value in values
                ],
            }
            for model in models
        ]
    }


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


def create_outcome(
    config: FalAIVideoConfig, model: str, provider: str | None, case: CreateCase
) -> JsonValue:
    raw = httpx.Response(
        status_code=200,
        content=case["body"].encode(),
        request=httpx.Request("POST", f"https://queue.fal.run/{model}"),
    )
    try:
        video = config.transform_video_create_response(
            model,
            raw,
            None,
            custom_llm_provider=provider,
            request_data=case["request_data"],
        )
    except ValueError:
        return {"error": {"status": None}}
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
            create_outcome(config, call["model"], provider, case)
            for provider in call["providers"]
            for case in call["cases"]
        ],
        "errors": [error_fields(config, case) for case in call["errors"]],
    }


def key_outcome(config: FalAIVideoConfig, case: KeyCase) -> JsonValue:
    try:
        headers = config.validate_environment(
            dict(case["headers"]),
            "",
            api_key=case["api_key"],
            litellm_params=GenericLiteLLMParams(api_key=case["litellm_api_key"]),
        )
    except ValueError:
        return {"error": {"status": None}}
    return {"headers": headers}


def queue_environment(
    config: FalAIVideoConfig, api_bases: list[str | None], cases: list[KeyCase]
) -> JsonValue:
    return {
        "urls": [
            config.get_complete_url("", api_base, {}) for api_base in api_bases
        ],
        "headers": [key_outcome(config, case) for case in cases],
    }


def queue_report(config: FalAIVideoConfig, call: QueueBaseLiteral) -> JsonValue:
    return {
        "environments": [
            environment_report(config, environment, call)
            for environment in call["environments"]
        ]
    }


def environment_report(
    config: FalAIVideoConfig, environment: dict[str, str], call: QueueBaseLiteral
) -> JsonValue:
    apply_environment(environment)
    return queue_environment(config, call["api_bases"], call["cases"])


def status_outcome(
    config: FalAIVideoConfig, video_id: str, api_base: str
) -> JsonValue:
    try:
        url = config.transform_video_status_retrieve_request(
            video_id, api_base, GenericLiteLLMParams(), {}
        )[0]
    except ValueError:
        return {"error": {"status": None}}
    return {"url": url}


def content_outcome(
    config: FalAIVideoConfig, video_id: str, api_base: str
) -> JsonValue:
    try:
        url = config.transform_video_content_request(
            video_id, api_base, GenericLiteLLMParams(), {}
        )[0]
    except ValueError:
        return {"error": {"status": None}}
    return {"url": url}


def job_outcome(
    config: FalAIVideoConfig, api_base: str | None, job: JobCase
) -> JsonValue:
    base = config.get_complete_url("", api_base, {})
    video_id = encode_video_id_with_provider(job["request_id"], PROVIDER, job["model"])
    return {
        "status": status_outcome(config, video_id, base),
        "content": content_outcome(config, video_id, base),
    }


def job_report(config: FalAIVideoConfig, call: WorkflowJobPath) -> JsonValue:
    return {
        "jobs": [
            job_outcome(config, api_base, job)
            for api_base in call["api_bases"]
            for job in call["jobs"]
        ]
    }


def run(input_value: Input) -> JsonValue:
    config = FalAIVideoConfig()
    if input_value["operation"] == "h3_auto_duration":
        return mapping_report(
            config, input_value["models"], "seconds", input_value["seconds"]
        )
    if input_value["operation"] == "seedance_decimal_duration":
        return mapping_report(
            config, input_value["models"], "seconds", input_value["seconds"]
        )
    if input_value["operation"] == "resolution_short_side":
        return mapping_report(
            config, input_value["models"], "size", input_value["sizes"]
        )
    if input_value["operation"] == "reference_type":
        return mapping_report(
            config,
            input_value["models"],
            "input_reference",
            input_value["references"],
        )
    if input_value["operation"] == "provider_override":
        return provider_report(config, input_value)
    if input_value["operation"] == "queue_base_literal":
        return queue_report(config, input_value)
    return job_report(config, input_value)
