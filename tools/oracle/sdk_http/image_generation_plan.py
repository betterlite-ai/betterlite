from typing import ClassVar

from not_built import Unbuilt


class ImageGenerationPlanRequest(Unbuilt):
    task: ClassVar[str] = "B1121"

    image_generation_plan: None
