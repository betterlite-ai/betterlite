from litellm.llms.bedrock.embed.amazon_titan_g1_transformation import AmazonTitanG1Config
from litellm.llms.bedrock.embed.amazon_titan_v2_transformation import AmazonTitanV2Config

from .support import ConfigProbe


# the probe attributes cover what get_config must drop: dunders, None, and every callable kind
class G1Probe(AmazonTitanG1Config):
    __hidden__ = "dunder"
    _retained = "kept"
    _count = 3
    _absent = None
    _builtin = len

    def _function(self) -> None: ...

    @classmethod
    def _class_method(cls) -> None: ...

    @staticmethod
    def _static_method() -> None: ...


class V2Probe(AmazonTitanV2Config):
    __hidden__ = "dunder"
    _retained = "kept"
    _count = 3
    _absent = None
    _builtin = len

    def _function(self) -> None: ...

    @classmethod
    def _class_method(cls) -> None: ...

    @staticmethod
    def _static_method() -> None: ...


G1_CONFIG: type[ConfigProbe] = G1Probe
V2_CONFIG: type[ConfigProbe] = V2Probe
