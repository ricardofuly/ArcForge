"""CI-only: sign draft release packages; keep credentials out of output/arguments."""
import base64
import os
from pathlib import Path
import re
import subprocess
import tempfile


def main():
    version = re.search(r'^version\s*=\s*"([^"]+)"', Path("src-tauri/Cargo.toml").read_text(), re.M)[1]
    tag = f"v{version}"
    public_key = os.environ["ARCFORGE_UPDATE_PUBLIC_KEY"].strip()
    with tempfile.TemporaryDirectory(prefix="arcforge-sign-") as temporary:
        root = Path(temporary)
        key = root / "private.key"
        # Exclusive creation and mode 0600, never stored in the repository or uploaded.
        with os.fdopen(os.open(key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as file:
            file.write(base64.b64decode(os.environ["ARCFORGE_SIGNING_KEY_BASE64"], validate=True))
        assets = root / "assets"
        assets.mkdir()
        subprocess.run(["gh", "release", "download", tag, "--dir", str(assets)], check=True)
        packages = [p for p in assets.iterdir() if p.suffix.lower() in {".exe", ".msi", ".appimage", ".deb", ".rpm"}]
        if not packages:
            raise RuntimeError("No packages in draft release; refusing publication")
        password = (os.environ.get("ARCFORGE_SIGNING_PASSWORD", "") + "\n").encode()
        signatures = []
        for package in packages:
            comment = f"ArcForge version={version} asset={package.name}"
            sign_args = ["minisign", "-S", "-s", str(key), "-m", str(package), "-t", comment]
            if not os.environ.get("ARCFORGE_SIGNING_PASSWORD"):
                sign_args.append("-W")
            subprocess.run(sign_args,
                           input=password, check=True, stdout=subprocess.DEVNULL)
            subprocess.run(["minisign", "-V", "-P", public_key, "-m", str(package)], check=True)
            signatures.append(str(package) + ".minisig")
        subprocess.run(["gh", "release", "upload", tag, *signatures, "--clobber"], check=True)
        subprocess.run(["gh", "release", "edit", tag, "--draft=false"], check=True)


if __name__ == "__main__":
    main()
