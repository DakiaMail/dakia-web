"""Generate browser/install icons from the original website PNG. Requires Pillow."""
import hashlib
from pathlib import Path
import shutil
import sys
from PIL import Image

EXPECTED = "1b003fd4d98cffeb70f3b45606e974227a8028c29f8398a9136c6da083c5c8c2"
public = Path(__file__).resolve().parents[1] / "public"
source = Path(sys.argv[1]) if len(sys.argv) > 1 else public / "dakia-icon-512.png"
if hashlib.sha256(source.read_bytes()).hexdigest() != EXPECTED:
    raise SystemExit("Expected the original apps/site/public/dakia-icon.png artwork")
if source.resolve() != (public / "dakia-icon-512.png").resolve():
    shutil.copyfile(source, public / "dakia-icon-512.png")
with Image.open(source) as image:
    for filename, size in [("favicon-32.png", 32), ("apple-touch-icon.png", 180), ("dakia-icon-192.png", 192)]:
        image.resize((size, size), Image.Resampling.LANCZOS).save(public / filename)
    image.save(public / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])
    # Preserve the entire authentic artwork inside the maskable safe circle.
    maskable = Image.new("RGB", (512, 512), "white")
    mark = image.convert("RGBA").resize((280, 280), Image.Resampling.LANCZOS)
    maskable.paste(mark, (116, 116), mark)
    maskable.save(public / "dakia-maskable-512.png")
print("Original website PNG preserved; favicon, touch and install icons generated.")
