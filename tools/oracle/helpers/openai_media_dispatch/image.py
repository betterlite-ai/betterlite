import base64
import json

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
from pydantic import JsonValue

from .cases import (
    ImageCases,
    ImageEditCase,
    ImageFile,
    ImageGenerationCase,
    ImageVariationCase,
)
from .support import OBJECT, OBJECTS, TEXT, TIMEOUT

litellm.suppress_debug_info = True


def datum_value(item: JsonValue) -> JsonValue:
    datum = OBJECT.validate_python(item)
    return {"b64_json": datum["b64_json"], "url": datum["url"]}


def image_reply(value: JsonValue) -> JsonValue:
    fields = OBJECT.validate_python(value)
    data = OBJECTS.validate_python(fields["data"])
    rendered: list[JsonValue] = []
    for item in data:
        rendered.append(datum_value(item))
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


def image_answer(outcome: dict[str, JsonValue], request: dict[str, JsonValue]) -> JsonValue:
    error = outcome["error"]
    if error is None:
        return {"request": request, "reply": image_reply(outcome["result"]), "error": None}
    return {"request": request, "reply": None, "error": image_error(error)}


def file_specs(files: list[ImageFile]) -> list[FileSpec | None]:
    specs: list[FileSpec | None] = []
    for file in files:
        encoded = base64.b64encode(file["content"].encode()).decode("ascii")
        specs.append(FileSpec(content_base64=encoded, name=file["name"]))
    return specs


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
    return image_answer(outcome, request)


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
        parts.append(
            {
                "field": fields["name"],
                "filename": fields["filename"],
                "content_type": fields["content_type"],
                "body": base64.b64decode(TEXT.validate_python(fields["content_base64"])).decode(),
            }
        )
    return {"url": outcome["url"], "data": outcome["data"], "parts": parts}


def image_variation(case: ImageVariationCase) -> JsonValue:
    probe = VariationProbe()
    probe.model = case["model"]
    probe.image_filename = case["image_filename"]
    probe.image_content = case["image_content"]
    probe.image_content_type = case["image_content_type"]
    probe.optional_params = case["options"]
    probe.api_key = "sk-variation"
    probe.api_base = case["api_base"]
    probe.timeout = TIMEOUT
    probe.reply = VariationReply(status=case["status"], body=json.dumps(case["reply"]))
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
    return image_answer(outcome, request)


def image_cases(case: ImageCases) -> JsonValue:
    match case["case"]:
        case "image_generation":
            return image_generation(case)
        case "image_edit":
            return image_edit(case)
        case "image_variation":
            return image_variation(case)
