import asyncio
from typing import TYPE_CHECKING, TypeAlias, cast

import httpx
from litellm.proxy.guardrails.guardrail_hooks.ibm_guardrails.ibm_detector import (
    IBMGuardrailDetector,
)
from litellm.types.guardrails import GuardrailEventHooks

if TYPE_CHECKING:
    from litellm.types.proxy.guardrails.guardrail_hooks.ibm import IBMDetectorDetection

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

GUARDRAIL_NAME = "ibm-detector"
TEST_BASE_URL = "https://ibm.test"
FAILED_TO_RESPOND = "guardrail_failed_to_respond"
INTERVENED = "guardrail_intervened"
SUCCESS = "success"


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def mapping(value: Json | None) -> dict[str, Json]:
    return {} if value is None else object_of(value)


def text_or_none(value: Json | None) -> str | None:
    return None if value is None else cast(str, value)


def number_or_none(value: Json | None) -> float | None:
    return None if value is None else cast(float, value)


def boolean(value: Json | None, *, default: bool) -> bool:
    return default if value is None else cast(bool, value)


def settings_of(input_value: dict[str, Json]) -> dict[str, Json]:
    return mapping(input_value.get("settings"))


def guardrail(settings: dict[str, Json]) -> IBMGuardrailDetector:
    optional = mapping(settings.get("optional_params"))
    return IBMGuardrailDetector(
        guardrail_name=GUARDRAIL_NAME,
        auth_token=text_or_none(settings.get("auth_token")),
        base_url=text_or_none(settings.get("base_url")),
        detector_id=text_or_none(settings.get("detector_id")),
        is_detector_server=boolean(settings.get("is_detector_server"), default=True),
        detector_params=mapping(optional.get("detector_params")),
        extra_headers=cast(dict[str, str], mapping(optional.get("extra_headers"))),
        score_threshold=number_or_none(optional.get("score_threshold")),
        block_on_detection=boolean(optional.get("block_on_detection"), default=True),
        verify_ssl=boolean(settings.get("verify_ssl"), default=True),
    )


def configure(fields: dict[str, Json]) -> Json:
    instance = guardrail(settings_of(fields))
    return {
        "guardrail_name": instance.guardrail_name,
        "auth_token_present": bool(instance.auth_token),
        "base_url": instance.base_url,
        "api_url": instance.api_url,
        "detector_id": instance.detector_id,
        "is_detector_server": instance.is_detector_server,
        "detector_params": cast(Json, instance.detector_params),
        "extra_headers": cast(Json, instance.extra_headers),
        "score_threshold": instance.score_threshold,
        "block_on_detection": instance.block_on_detection,
        "verify_ssl": instance.verify_ssl,
    }


def mock_transport(
    answer: Json, captured: list[Json], header_names: list[str]
) -> httpx.MockTransport:
    def handle(request: httpx.Request) -> httpx.Response:
        captured.append(
            {
                "path": request.url.path,
                "body": cast(Json, httpx.Response(200, content=request.content).json()),
                "headers": {name: request.headers.get(name) for name in header_names},
            }
        )
        match answer:
            case {"status": int(status), "body": body}:
                return httpx.Response(status, json=body)
            case _:
                raise httpx.ReadTimeout("the mock never answers", request=request)

    return httpx.MockTransport(handle)


async def scan(instance: IBMGuardrailDetector, contents: list[str]) -> tuple[Json, str]:
    if instance.is_detector_server:
        result = await instance._call_detector_server(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            contents=contents, event_type=GuardrailEventHooks.pre_call
        )
        if not isinstance(result, list):  # pyright: ignore[reportUnnecessaryIsInstance]
            raise TypeError("the detector server response is not a list")
        return cast(Json, result), cast(
            str,
            instance._determine_guardrail_status_detector_server(result),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        )
    results: list[Json] = []
    statuses: list[str] = []
    for content in contents:
        result = await instance._call_orchestrator(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            content=content, event_type=GuardrailEventHooks.pre_call
        )
        results.append(cast(Json, result))
        statuses.append(
            cast(
                str,
                instance._determine_guardrail_status_orchestrator({"detections": result}),  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            )
        )
    return results, INTERVENED if INTERVENED in statuses else SUCCESS


def detect(fields: dict[str, Json]) -> Json:
    settings = settings_of(fields)
    settings["base_url"] = TEST_BASE_URL
    instance = guardrail(settings)
    contents = cast(list[str], fields["contents"])
    captured: list[Json] = []
    extra_headers = instance.extra_headers
    header_names = [
        "content-type",
        "detector-id",
        "authorization",
        *(name.lower() for name in extra_headers),
    ]
    instance.async_handler.client = httpx.AsyncClient(
        transport=mock_transport(fields.get("answer"), captured, header_names)
    )
    try:
        detections, status = asyncio.run(scan(instance, contents))
    except Exception:  # noqa: BLE001 - any transport or shape failure is a failed response
        return {"requests": captured, "detections": None, "status": FAILED_TO_RESPOND}
    return {"requests": captured, "detections": detections, "status": status}


def filter_detections(fields: dict[str, Json]) -> Json:
    instance = guardrail(settings_of(fields))
    detections = cast(
        "list[IBMDetectorDetection]", fields["detections"]
    )
    filtered = instance._filter_detections_by_threshold(detections)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
    return cast(Json, filtered)


def message_detector_server(fields: dict[str, Json]) -> Json:
    instance = guardrail(settings_of(fields))
    detections = cast(
        "list[list[IBMDetectorDetection]]", fields["detections"]
    )
    return instance._create_error_message_detector_server(detections)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]


def message_orchestrator(fields: dict[str, Json]) -> Json:
    instance = guardrail(settings_of(fields))
    detections = cast(
        "list[IBMDetectorDetection]", fields["detections"]
    )
    return instance._create_error_message_orchestrator(detections)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]


def run(input_value: dict[str, Json]) -> Json:
    [(operation, request)] = input_value.items()
    fields = object_of(request)
    match operation:
        case "configure":
            return configure(fields)
        case "detect":
            return detect(fields)
        case "filter":
            return filter_detections(fields)
        case "message_detector_server":
            return message_detector_server(fields)
        case "message_orchestrator":
            return message_orchestrator(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
