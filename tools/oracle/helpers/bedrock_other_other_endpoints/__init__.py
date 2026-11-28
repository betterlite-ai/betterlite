import litellm
from pydantic import JsonValue

from .common import JSON
from .count import count_row
from .embedding import embedding_row, status_row
from .image import image_row
from .models import INPUT, Probe
from .rerank import rerank_row
from .transcription import transcription_row

litellm.suppress_debug_info = True


def probe_row(model_cost: dict[str, JsonValue] | None, probe: Probe) -> JsonValue:
    row: JsonValue = None
    match probe["case"]:
        case "embedding":
            row = embedding_row(probe)
        case "embedding_status":
            row = status_row(probe)
        case "rerank":
            row = rerank_row(probe)
        case "count":
            row = count_row(probe)
        case "transcription":
            row = transcription_row(probe)
        case "image_generation":
            row = image_row(model_cost, probe, "canvas_image")
        case "image_edit":
            row = image_row(model_cost, probe, "canvas_edit")
    return row


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    values = INPUT.validate_python(input_value)
    model_cost = values.get("model_cost")
    rows: list[JsonValue] = [
        probe_row(model_cost, probe) for probe in values["probes"]
    ]
    return JSON.validate_python(rows)
