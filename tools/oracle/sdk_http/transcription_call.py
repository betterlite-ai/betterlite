from typing import ClassVar

from not_built import Unbuilt


class TranscriptionCallRequest(Unbuilt):
    task: ClassVar[str] = "B1082"

    transcription_call: None
