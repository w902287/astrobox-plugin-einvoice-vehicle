#!/usr/bin/env python3
"""Fail CI when tracked files contain common secrets or private image metadata."""
import re
import subprocess
import sys
from pathlib import Path
from PIL import Image

files = [Path(x) for x in subprocess.check_output(["git", "ls-files"], text=True).splitlines()]
name_block = re.compile(r"(^|/)(\.env($|\.)|[^/]+\.(pem|p12|pfx|mobileprovision))$", re.I)
secret_patterns = {
    "private key": re.compile(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----"),
    "GitHub token": re.compile(r"(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,})"),
    "AWS access key": re.compile(r"AKIA[0-9A-Z]{16}"),
    "Bearer token": re.compile(r"Bearer\s+[A-Za-z0-9._~+/-]{24,}={0,2}", re.I),
    "generic secret assignment": re.compile(r"(?:api[_-]?key|secret|password|auth[_-]?token)\s*[:=]\s*['\"][^'\"]{12,}['\"]", re.I),
}
errors = []
for path in files:
    name = path.as_posix()
    if name_block.search(name):
        errors.append(f"forbidden private filename: {name}")
    if path.suffix.lower() in {".png", ".jpg", ".jpeg", ".webp"}:
        try:
            image = Image.open(path)
            hidden = {"exif", "xmp", "XML:com.adobe.xmp", "Raw profile type exif"} & set(image.info)
            if hidden or image.getexif():
                errors.append(f"private image metadata: {name} ({sorted(hidden)})")
        except Exception as exc:
            errors.append(f"cannot inspect image {name}: {exc}")
    try:
        text = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError):
        continue
    for label, pattern in secret_patterns.items():
        if pattern.search(text):
            errors.append(f"{label}: {name}")
if errors:
    print("Privacy scan failed:")
    print("\n".join(f"- {item}" for item in errors))
    sys.exit(1)
print(f"Privacy scan passed: {len(files)} tracked files checked")
