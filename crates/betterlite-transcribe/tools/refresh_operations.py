import json
from pathlib import Path

import botocore.session


ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    operations = botocore.session.get_session().get_service_model("transcribe").operation_names
    (ROOT / "data" / "operations.txt").write_text("".join(f"{name}\n" for name in sorted(operations)))


if __name__ == "__main__":
    main()
