from typing import ClassVar

from not_built import Unbuilt


class SpeechCallPlanRequest(Unbuilt):
    task: ClassVar[str] = "B1083"

    speech_call_plan: None
