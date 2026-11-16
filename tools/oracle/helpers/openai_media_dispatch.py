import base64
import json
from typing import Annotated, Final, Literal, TypeAlias, TypedDict

import litellm
from openai_image_edits import (
    FileSpec,
    Probe as ImageEditProbe,
    transform_value as image_edit_outcome,
)
from openai_image_generation.probe import Answer as ImageAnswer, Probe as ImageProbe
from openai_image_generation.request import request_value as image_generation_outcome
from openai_image_variations.probe import Probe as VariationProbe, Reply as VariationReply
from openai_image_variations.request import request_value as variation_outcome
from openai_speech import run as speech_outcome
from openai_speech.types import (
    ClientSpec,
    Input as SpeechInput,
    Reply as SpeechReply,
    Request as SpeechRequest,
)
from openai_transcription import exchange as transcription_outcome
from openai_transcription.probe import Answer as TranscriptionAnswer, Probe as TranscriptionProbe
from openai_video_edits import (
    EditBody,
    Fixture,
    create_character_reply,
    create_character_value,
    edit_reply,
    edit_value,
    extension_reply,
    extension_value,
    get_character_reply,
    get_character_value,
)
from openai_video_edits.fixtures import BytesFixture
from openai_videos import CONFIG as VIDEO_CONFIG, create_ops, reply_ops, request_ops
from openai_videos.views import (
    BytesReference,
    ContentReplyCall,
    ContentRequestCall,
    CreateCall,
    CreateReplyCall,
    DeleteReplyCall,
    DeleteRequestCall,
    ListReplyCall,
    ListRequestCall,
    Reference,
    RemixReplyCall,
    RemixRequestCall,
    StatusReplyCall,
    StatusRequestCall,
)
from pydantic import Field, JsonValue, TypeAdapter

litellm.suppress_debug_info = True

MODEL: Final = "sora-2"
TIMEOUT: Final = 60
SPEECH_MAX_RETRIES: Final = 0

OBJECT: Final = TypeAdapter(dict[str, JsonValue])
OBJECTS: Final = TypeAdapter(list[JsonValue])
TEXT: Final = TypeAdapter(str)
BYTES: Final = TypeAdapter(bytes)
EDIT_BODY: Final = TypeAdapter(EditBody)


class ImageFile(TypedDict):
    content: str
    name: str | None


class ImageGenerationCase(TypedDict):
    case: Literal["image_generation"]
    api_base: str
    model: str
    prompt: str
    options: dict[str, JsonValue]
    status: int
    reply: JsonValue


class ImageEditCase(TypedDict):
    case: Literal["image_edit"]
    api_base: str
    model: str
    prompt: str | None
    image: list[ImageFile]
    mask: list[ImageFile] | None
    options: dict[str, JsonValue]


class ImageVariationCase(TypedDict):
    case: Literal["image_variation"]
    api_base: str
    model: str
    image_filename: str | None
    image_content: str
    image_content_type: str | None
    options: dict[str, JsonValue]
    status: int
    reply: JsonValue


class SpeechOptions(TypedDict, total=False):
    response_format: str
    speed: float
    instructions: str
    extra_headers: dict[str, str]


class SpeechCase(TypedDict):
    case: Literal["speech"]
    api_base: str
    model: str
    input: str
    voice: str
    options: SpeechOptions
    status: int
    content_type: str | None
    body: str


class TranscriptionCase(TypedDict):
    case: Literal["transcription"]
    api_base: str
    model: str
    family: Literal["whisper", "gpt"]
    options: dict[str, JsonValue]
    status: int
    reply: str


class VideoCreateCase(TypedDict):
    case: Literal["video_create"]
    api_base: str
    model: str
    prompt: str
    options: dict[str, JsonValue]
    reference: str | None
    provider: str | None
    reply: str


class VideoLifecycleCase(TypedDict):
    case: Literal["video_lifecycle"]
    api_base: str
    provider: str | None
    op: str
    video_id: str | None
    after: str | None
    limit: int | None
    order: str | None
    variant: str | None
    prompt: str | None
    reply: str


class VideoExtendedCase(TypedDict):
    case: Literal["video_extended_operations"]
    api_base: str
    provider: str | None
    model: str
    op: str
    prompt: str | None
    video_id: str | None
    video_file: str | None
    extra_body: dict[str, JsonValue] | None
    seconds: str | None
    name: str | None
    video: str | None
    character_id: str | None
    reply: str


MediaCase: TypeAlias = Annotated[
    (
        ImageGenerationCase
        | ImageEditCase
        | ImageVariationCase
        | SpeechCase
        | TranscriptionCase
        | VideoCreateCase
        | VideoLifecycleCase
        | VideoExtendedCase
    ),
    Field(discriminator="case"),
]
CASES: Final = TypeAdapter(MediaCase)

ImageCases: TypeAlias = ImageGenerationCase | ImageEditCase | ImageVariationCase
AudioCases: TypeAlias = SpeechCase | TranscriptionCase
VideoCases: TypeAlias = VideoCreateCase | VideoLifecycleCase | VideoExtendedCase


def required(value: str | None, name: str) -> str:
    if value is None:
        raise ValueError(f"the case needs {name}")
    return value


def image_reply(value: JsonValue) -> JsonValue:
    fields = OBJECT.validate_python(value)
    data = OBJECTS.validate_python(fields["data"])
    rendered: list[JsonValue] = []
    for item in data:
        datum = OBJECT.validate_python(item)
        rendered.append({"b64_json": datum["b64_json"], "url": datum["url"]})
    return {
        "created": fields["created"],
        "background": fields["background"],
        "quality": fields["quality"],
        "output_format": fields["output_format"],
        "size": fields["size"],
        "data": rendered,
    }


def image_error(value: JsonValue) -> JsonValue:
    fields = OBJECT.validate_python(value)
    return {"mapped": fields["mapped"], "status": fields["status"]}


def file_specs(files: list[ImageFile]) -> list[FileSpec | None]:
    specs: list[FileSpec | None] = []
    for file in files:
        encoded = base64.b64encode(file["content"].encode()).decode("ascii")
        specs.append(FileSpec(content_base64=encoded, name=file["name"]))
    return specs


def leaf_parts(files: list[JsonValue]) -> list[JsonValue]:
    parts: list[JsonValue] = []
    for part in files:
        fields = OBJECT.validate_python(part)
        body = TEXT.validate_python(fields["body"])
        if fields["body_kind"] == "bytes":
            body = base64.b64decode(body).decode()
        parts.append(
            {
                "field": fields["field"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": body,
            }
        )
    return parts


def image_generation(case: ImageGenerationCase) -> JsonValue:
    probe = ImageProbe(
        model=case["model"],
        prompt=case["prompt"],
        optional_params=case["options"],
        api_key="sk-media",
        api_base=case["api_base"],
        timeout=TIMEOUT,
        reply=ImageAnswer(status=case["status"], body=json.dumps(case["reply"])),
    )
    outcome = OBJECT.validate_python(image_generation_outcome(probe))
    call = OBJECT.validate_python(OBJECTS.validate_python(outcome["calls"])[0])
    headers = OBJECT.validate_python(call["headers"])
    request: dict[str, JsonValue] = {
        "method": "POST",
        "url": call["url"],
        "body": call["body"],
        "authorization": headers.get("authorization"),
    }
    error = outcome["error"]
    if error is None:
        return {"request": request, "reply": image_reply(outcome["result"]), "error": None}
    return {"request": request, "reply": None, "error": image_error(error)}


def image_edit(case: ImageEditCase) -> JsonValue:
    mask = case["mask"]
    probe: ImageEditProbe = {
        "model": case["model"],
        "prompt": case["prompt"],
        "image": file_specs(case["image"]),
        "mask": None if mask is None else file_specs(mask),
        "options": case["options"],
        "params": {"api_base": case["api_base"]},
        "globals": {},
        "environment": {},
        "reply": {"status": 200, "body": "{}"},
    }
    outcome = OBJECT.validate_python(image_edit_outcome(probe))
    parts: list[JsonValue] = []
    for part in OBJECTS.validate_python(outcome["files"]):
        fields = OBJECT.validate_python(part)
        encoded = TEXT.validate_python(fields["content_base64"])
        parts.append(
            {
                "field": fields["name"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": base64.b64decode(encoded).decode(),
            }
        )
    return {"url": outcome["url"], "data": outcome["data"], "parts": parts}


def image_variation(case: ImageVariationCase) -> JsonValue:
    probe = VariationProbe(
        model=case["model"],
        image_filename=case["image_filename"],
        image_content=case["image_content"],
        image_content_type=case["image_content_type"],
        optional_params=case["options"],
        api_key="sk-variation",
        api_base=case["api_base"],
        timeout=TIMEOUT,
        reply=VariationReply(status=case["status"], body=json.dumps(case["reply"])),
    )
    outcome = OBJECT.validate_python(variation_outcome(probe))
    call = OBJECT.validate_python(OBJECTS.validate_python(outcome["calls"])[0])
    parts: list[JsonValue] = []
    for part in OBJECTS.validate_python(call["parts"]):
        fields = OBJECT.validate_python(part)
        parts.append(
            {
                "name": fields["name"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": fields["data"],
            }
        )
    request: dict[str, JsonValue] = {
        "url": call["url"],
        "authorization": call["authorization"],
        "parts": parts,
    }
    error = outcome["error"]
    if error is None:
        return {"request": request, "reply": image_reply(outcome["result"]), "error": None}
    return {"request": request, "reply": None, "error": image_error(error)}


def speech_request(case: SpeechCase) -> SpeechRequest:
    options = case["options"]
    request: SpeechRequest = {
        "model": case["model"],
        "input": case["input"],
        "voice": case["voice"],
    }
    if "response_format" in options:
        request["response_format"] = options["response_format"]
    if "speed" in options:
        request["speed"] = options["speed"]
    if "instructions" in options:
        request["instructions"] = options["instructions"]
    if "extra_headers" in options:
        request["extra_headers"] = OBJECT.validate_python(options["extra_headers"])
    return request


def speech(case: SpeechCase) -> JsonValue:
    headers: list[list[str]] = []
    content_type = case["content_type"]
    if content_type is not None:
        headers.append(["content-type", content_type])
    client: ClientSpec = {
        "timeout": TIMEOUT,
        "max_retries": SPEECH_MAX_RETRIES,
        "api_key": "sk-speech",
        "api_base": case["api_base"],
        "aspeech": False,
    }
    reply: SpeechReply = {"status": case["status"], "headers": headers, "text": case["body"]}
    probe: SpeechInput = {"client": client, "request": speech_request(case), "reply": reply}
    outcome = OBJECT.validate_python(speech_outcome(probe))
    sent = OBJECT.validate_python(outcome["request"])
    request: dict[str, JsonValue] = {
        "method": sent["method"],
        "url": sent["url"],
        "accept": sent["accept"],
        "authorization": sent["authorization"],
        "extra_headers": sent["extra_headers"],
        "body": sent["body"],
    }
    result = OBJECT.validate_python(outcome["result"])
    raised = result.get("raised")
    if raised is not None:
        error = OBJECT.validate_python(raised)
        return {
            "request": request,
            "status": None,
            "content_type": None,
            "body": None,
            "error": {
                "status": error["status"],
                "type": error["type"],
                "param": error["param"],
                "code": error["code"],
            },
        }
    content = BYTES.validate_python(result["body"])
    return {
        "request": request,
        "status": result["status"],
        "content_type": result["content_type"],
        "body": content.decode(),
        "error": None,
    }


def transcription(case: TranscriptionCase) -> JsonValue:
    probe = TranscriptionProbe(
        model=case["model"],
        provider=case["family"],
        optional_params=case["options"],
        headers={},
        api_key="sk-transcription",
        api_base=case["api_base"],
        reply=TranscriptionAnswer(status=case["status"], body=case["reply"]),
    )
    outcome = OBJECT.validate_python(transcription_outcome(probe))
    sent = OBJECT.validate_python(outcome["request"])
    parts: list[JsonValue] = []
    for part in OBJECTS.validate_python(sent["parts"]):
        fields = OBJECT.validate_python(part)
        encoded = TEXT.validate_python(fields["body"])
        parts.append(
            {
                "name": fields["name"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": base64.b64decode(encoded).decode(),
            }
        )
    request: dict[str, JsonValue] = {
        "url": sent["url"],
        "authorization": sent["authorization"],
        "accept": sent["accept"],
        "parts": parts,
    }
    error = outcome["error"]
    if error is not None:
        raised = OBJECT.validate_python(error)
        return {
            "request": request,
            "reply": None,
            "error": {
                "status": raised["status"],
                "type": raised["type"],
                "param": raised["param"],
                "code": raised["code"],
            },
        }
    result = OBJECT.validate_python(outcome["result"])
    response = OBJECT.validate_python(result["response"])
    return {
        "request": request,
        "reply": {
            "text": response["text"],
            "fields": response["fields"],
            "model": result["model"],
            "provider": result["provider"],
            "duration_recorded": result["duration_recorded"],
        },
        "error": None,
    }


def video_create(case: VideoCreateCase) -> JsonValue:
    base = TEXT.validate_python(VIDEO_CONFIG.get_complete_url(case["model"], case["api_base"], {}))
    reference: Reference | None = None
    given = case["reference"]
    if given is not None:
        reference = BytesReference(
            kind="bytes", data_base64=base64.b64encode(given.encode()).decode("ascii")
        )
    create_call: CreateCall = {
        "op": "create",
        "model": case["model"],
        "prompt": case["prompt"],
        "api_base": base,
        "options": case["options"],
        "reference": reference,
    }
    created = OBJECT.validate_python(create_ops(create_call))
    create_reply: CreateReplyCall = {
        "op": "create_response",
        "model": case["model"],
        "provider": case["provider"],
        "body_base64": base64.b64encode(case["reply"].encode()).decode("ascii"),
    }
    reply = create_ops(create_reply)
    return {
        "url": created["api_base"],
        "body": created["data"],
        "parts": leaf_parts(OBJECTS.validate_python(created["files"])),
        "multipart": created["multipart"],
        "reply": reply,
    }


def video_lifecycle(case: VideoLifecycleCase) -> JsonValue:
    base = TEXT.validate_python(VIDEO_CONFIG.get_complete_url(MODEL, case["api_base"], {}))
    op = case["op"]
    method = "GET"
    params: JsonValue = {}
    body: JsonValue = None
    reply: JsonValue = None
    encoded = base64.b64encode(case["reply"].encode()).decode("ascii")
    if op == "list":
        list_call: ListRequestCall = {
            "op": "list_request",
            "api_base": base,
            "after": case["after"],
            "limit": case["limit"],
            "order": case["order"],
        }
        sent = OBJECT.validate_python(request_ops(list_call))
        params = sent["params"]
        list_reply: ListReplyCall = {
            "op": "list_response",
            "provider": case["provider"],
            "body_base64": encoded,
        }
        reply = reply_ops(list_reply)
    elif op == "status":
        status_call: StatusRequestCall = {
            "op": "status_request",
            "video_id": required(case["video_id"], "video_id"),
            "api_base": base,
        }
        sent = OBJECT.validate_python(request_ops(status_call))
        status_reply: StatusReplyCall = {
            "op": "status_response",
            "provider": case["provider"],
            "body_base64": encoded,
        }
        reply = reply_ops(status_reply)
    elif op == "delete":
        delete_call: DeleteRequestCall = {
            "op": "delete_request",
            "video_id": required(case["video_id"], "video_id"),
            "api_base": base,
        }
        sent = OBJECT.validate_python(request_ops(delete_call))
        method = "DELETE"
        delete_reply: DeleteReplyCall = {"op": "delete_response", "body_base64": encoded}
        reply = reply_ops(delete_reply)
    elif op == "content":
        content_call: ContentRequestCall = {
            "op": "content_request",
            "video_id": required(case["video_id"], "video_id"),
            "api_base": base,
            "variant": case["variant"],
        }
        sent = OBJECT.validate_python(request_ops(content_call))
        content_reply: ContentReplyCall = {"op": "content_response", "body_base64": encoded}
        content = OBJECT.validate_python(reply_ops(content_reply))
        content_body: dict[str, JsonValue] = {
            "body": base64.b64decode(TEXT.validate_python(content["body_base64"])).decode()
        }
        reply = content_body
    elif op == "remix":
        remix_call: RemixRequestCall = {
            "op": "remix_request",
            "video_id": required(case["video_id"], "video_id"),
            "prompt": required(case["prompt"], "prompt"),
            "api_base": base,
        }
        sent = OBJECT.validate_python(request_ops(remix_call))
        method = "POST"
        body = sent["params"]
        remix_reply: RemixReplyCall = {
            "op": "remix_response",
            "provider": case["provider"],
            "body_base64": encoded,
        }
        reply = reply_ops(remix_reply)
    else:
        raise ValueError(f"unknown video op {op}")
    return {
        "request": {
            "method": method,
            "url": sent["url"],
            "params": params,
            "body": body,
            "parts": [],
            "multipart": False,
        },
        "reply": reply,
    }


def video_extended(case: VideoExtendedCase) -> JsonValue:
    base = TEXT.validate_python(VIDEO_CONFIG.get_complete_url(case["model"], case["api_base"], {}))
    op = case["op"]
    body: JsonValue = None
    parts: list[JsonValue] = []
    multipart = False
    reply: JsonValue = None
    extra = case["extra_body"]
    if op == "edit":
        fixture: Fixture | None = None
        given = case["video_file"]
        if given is not None:
            fixture = BytesFixture(
                kind="bytes", data_base64=base64.b64encode(given.encode()).decode("ascii")
            )
        sent = OBJECT.validate_python(
            edit_value(
                {
                    "op": "edit_request",
                    "prompt": required(case["prompt"], "prompt"),
                    "video_id": required(case["video_id"], "video_id"),
                    "api_base": base,
                    "fixture": fixture,
                    "extra_body": None if extra is None else EDIT_BODY.validate_python(extra),
                    "prefetched_source_data": None,
                }
            )
        )
        body = sent["body"]
        files = sent["files"]
        if files is not None:
            parts = leaf_parts(OBJECTS.validate_python(files))
            multipart = True
        reply = edit_reply(
            {
                "op": "edit_response",
                "body": case["reply"],
                "provider": case["provider"],
                "request_data": None,
            }
        )
    elif op == "extension":
        sent = OBJECT.validate_python(
            extension_value(
                {
                    "op": "extension_request",
                    "prompt": required(case["prompt"], "prompt"),
                    "video_id": required(case["video_id"], "video_id"),
                    "seconds": required(case["seconds"], "seconds"),
                    "api_base": base,
                    "extra_body": None if extra is None else EDIT_BODY.validate_python(extra),
                }
            )
        )
        body = sent["body"]
        reply = extension_reply(
            {"op": "extension_response", "body": case["reply"], "provider": case["provider"]}
        )
    elif op == "create_character":
        video = required(case["video"], "video")
        sent = OBJECT.validate_python(
            create_character_value(
                {
                    "op": "create_character_request",
                    "name": required(case["name"], "name"),
                    "fixture": BytesFixture(
                        kind="bytes",
                        data_base64=base64.b64encode(video.encode()).decode("ascii"),
                    ),
                    "api_base": base,
                }
            )
        )
        parts = leaf_parts(OBJECTS.validate_python(sent["files"]))
        multipart = True
        reply = create_character_reply({"op": "create_character_response", "body": case["reply"]})
    elif op == "get_character":
        sent = OBJECT.validate_python(
            get_character_value(
                {
                    "op": "get_character_request",
                    "character_id": required(case["character_id"], "character_id"),
                    "api_base": base,
                }
            )
        )
        reply = get_character_reply({"op": "get_character_response", "body": case["reply"]})
    else:
        raise ValueError(f"unknown video edit op {op}")
    return {
        "url": sent["url"],
        "body": body,
        "parts": parts,
        "multipart": multipart,
        "reply": reply,
    }


def image_cases(case: ImageCases) -> JsonValue:
    match case["case"]:
        case "image_generation":
            return image_generation(case)
        case "image_edit":
            return image_edit(case)
        case "image_variation":
            return image_variation(case)


def audio_cases(case: AudioCases) -> JsonValue:
    match case["case"]:
        case "speech":
            return speech(case)
        case "transcription":
            return transcription(case)


def video_cases(case: VideoCases) -> JsonValue:
    match case["case"]:
        case "video_create":
            return video_create(case)
        case "video_lifecycle":
            return video_lifecycle(case)
        case "video_extended_operations":
            return video_extended(case)


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    spec = CASES.validate_python(input_value)
    match spec["case"]:
        case "image_generation" | "image_edit" | "image_variation":
            return image_cases(spec)
        case "speech" | "transcription":
            return audio_cases(spec)
        case "video_create" | "video_lifecycle" | "video_extended_operations":
            return video_cases(spec)
