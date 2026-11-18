from typing import ClassVar

from not_built import Unbuilt


class SpeechCallRequest(Unbuilt):
    task: ClassVar[str] = "B1084"

    speech_call: None
