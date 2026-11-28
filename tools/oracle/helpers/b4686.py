from pathlib import Path

import litellm

__path__ = [str(Path(__file__).with_suffix(""))]

from b4686.entry import run

litellm.suppress_debug_info = True

__all__ = ("run",)
