from pathlib import Path

import litellm

__path__ = [str(Path(__file__).with_suffix(""))]

from responses_ws_managed.support_pkg import run as run
from responses_ws_managed.support_pkg.fakes import Input as Input

litellm.suppress_debug_info = True
