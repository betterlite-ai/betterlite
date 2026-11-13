import litellm
from pydantic import JsonValue as Json

from voxtral_speech import TranscriptionInput, transcription_run


def run(input_value: TranscriptionInput) -> Json:
    litellm.suppress_debug_info = True
    return transcription_run(input_value)
