import litellm
from pydantic import JsonValue

from .audio import audio_cases
from .cases import CASES
from .image import image_cases
from .video import video_cases
from .video_edits import video_edit_cases

litellm.suppress_debug_info = True


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    spec = CASES.validate_python(input_value)
    match spec["case"]:
        case "image_generation" | "image_edit" | "image_variation":
            return image_cases(spec)
        case "speech" | "transcription":
            return audio_cases(spec)
        case "video_create" | "video_lifecycle":
            return video_cases(spec)
        case "video_extended_operations":
            return video_edit_cases(spec)
