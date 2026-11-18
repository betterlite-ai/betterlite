from typing import ClassVar

from not_built import Unbuilt


class ImageGenerationCallRequest(Unbuilt):
    task: ClassVar[str] = "B998"

    image_generation_call: None
