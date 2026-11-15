from typing import ClassVar

from pydantic import ConfigDict

from betterlite.types.llms.openai.chat.fine_tuning import Hyperparameters


class OpenAIFineTuningHyperparameters(Hyperparameters):  # ast-grep-ignore: no-long-names
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="allow")
