#!/usr/bin/env python3
"""Fetch the oversized public classifier at its pinned, hash-checked revision."""
import hashlib
import json
from pathlib import Path
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
ASSETS = ROOT / "crates/inference/assets/email-classifier-v2"


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def main():
    manifest = json.loads((ASSETS / "MANIFEST.json").read_text())
    target = ASSETS / "model.onnx"
    expected = manifest["files"][target.name]
    if target.is_file() and digest(target) == expected:
        target.chmod(0o644)
        print("Classifier already verified")
        return
    url = (f"https://huggingface.co/{manifest['model']}/resolve/"
           f"{manifest['revision']}/{target.name}")
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=ASSETS, prefix=".model-", delete=False) as output:
            temporary = Path(output.name)
            with urllib.request.urlopen(url, timeout=120) as response:
                while chunk := response.read(1024 * 1024):
                    output.write(chunk)
        if digest(temporary) != expected:
            raise ValueError("Classifier SHA-256 mismatch; existing model left unchanged")
        temporary.chmod(0o644)
        temporary.replace(target)
        print("Classifier downloaded and verified")
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
