#!/usr/bin/env python3
"""Fetch only public, pinned multilingual E5 assets. Never accepts mail inputs."""
import hashlib
import json
from pathlib import Path
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/inference/assets/multilingual-e5-small"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    manifest = json.loads((ASSETS / "MANIFEST.json").read_text())
    for name, expected in manifest["files"].items():
        target = ASSETS / name
        if target.is_file() and digest(target) == expected:
            continue
        remote = manifest["onnx_path"] if name == "model.onnx" else name
        url = (f"https://huggingface.co/{manifest['onnx_model']}/resolve/"
               f"{manifest['revision']}/{remote}")
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(dir=ASSETS, prefix=".download-", delete=False) as output:
                temporary = Path(output.name)
                with urllib.request.urlopen(url, timeout=120) as response:
                    while chunk := response.read(1024 * 1024):
                        output.write(chunk)
            if digest(temporary) != expected:
                raise ValueError(f"Search model SHA-256 mismatch: {name}; existing asset retained")
            temporary.chmod(0o644)
            temporary.replace(target)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
    print("Search model assets verified")


if __name__ == "__main__":
    main()
