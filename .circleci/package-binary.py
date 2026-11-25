"""Package the native release binary and write its SHA-256 checksum."""

import hashlib
import subprocess
import tarfile
import zipfile
from pathlib import Path


def package():
    version = subprocess.check_output(["rustc", "-vV"], text=True)
    target = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
    windows = "windows" in target
    binary = Path("target/release/opencl_vanity_gpg" + (".exe" if windows else ""))
    files = [binary, Path("LICENSE"), Path("README.md")]
    output = Path("target/artifacts")
    output.mkdir(parents=True, exist_ok=True)
    archive_path = output / (f"opencl_vanity_gpg-{target}" + (".zip" if windows else ".tar.gz"))
    if windows:
        with zipfile.ZipFile(archive_path, "w", compression=zipfile.ZIP_DEFLATED) as archive:
            for source in files:
                archive.write(source, source.name)
    else:
        with tarfile.open(archive_path, "w:gz") as archive:
            for source in files:
                archive.add(source, arcname=source.name)
    digest = hashlib.sha256(archive_path.read_bytes()).hexdigest()
    # Keep checksums portable: Windows text mode would translate LF to CRLF.
    archive_path.with_name(archive_path.name + ".sha256").write_bytes(
        f"{digest}  {archive_path.name}\n".encode("utf-8")
    )
    print(archive_path)


if __name__ == "__main__":
    package()
