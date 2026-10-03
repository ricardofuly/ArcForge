"""Exercise release gating without invoking GitHub or using real signing keys."""
import base64
import importlib.util
import os
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("sign_release", ROOT / "scripts/sign-release.py")
sign_release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sign_release)


class ReleaseSecurityTests(unittest.TestCase):
    def exercise(self, reject=False, password="dummy password"):
        calls = []
        key_paths = []

        def run(args, **kwargs):
            calls.append(args)
            if args[:3] == ["gh", "release", "download"]:
                destination = Path(args[args.index("--dir") + 1])
                (destination / "ArcForge-setup.exe").write_bytes(b"dummy installer")
                (destination / "ArcForge_0.1.4_windows_x64.bin").write_bytes(b"dummy executable")
            if args[:2] == ["minisign", "-S"]:
                key = Path(args[args.index("-s") + 1])
                key_paths.append(key)
                self.assertEqual(key.read_bytes(), b"dummy key")
                self.assertNotIn("dummy key", " ".join(args))
                self.assertEqual(kwargs["input"], (password + "\n").encode())
                self.assertEqual("-W" in args, not bool(password))
            if args[:2] == ["minisign", "-V"] and reject:
                raise subprocess.CalledProcessError(1, args)

        env = {
            "ARCFORGE_UPDATE_PUBLIC_KEY": "dummy public key",
            "ARCFORGE_SIGNING_KEY_BASE64": base64.b64encode(b"dummy key").decode(),
            "ARCFORGE_SIGNING_PASSWORD": password,
        }
        with patch.dict(os.environ, env), patch.object(sign_release.subprocess, "run", run):
            original = Path.cwd()
            try:
                os.chdir(ROOT)
                if reject:
                    with self.assertRaises(subprocess.CalledProcessError):
                        sign_release.main()
                else:
                    sign_release.main()
            finally:
                os.chdir(original)
        self.assertTrue(key_paths)
        self.assertTrue(all(not key.exists() for key in key_paths))
        return calls

    def test_invalid_signature_never_uploads_or_publishes(self):
        calls = self.exercise(reject=True)
        self.assertFalse(any(args[:3] in (["gh", "release", "upload"], ["gh", "release", "edit"]) for args in calls))

    def test_verification_precedes_upload_and_publication(self):
        calls = self.exercise()
        verify = next(i for i, args in enumerate(calls) if args[:2] == ["minisign", "-V"])
        upload = next(i for i, args in enumerate(calls) if args[:3] == ["gh", "release", "upload"])
        publish = next(i for i, args in enumerate(calls) if args[:3] == ["gh", "release", "edit"])
        live_verify = next(i for i, args in enumerate(calls) if args[:2] == ["minisign", "-V"] and args[-1].endswith(".bin"))
        self.assertLess(live_verify, upload)
        self.assertLess(verify, upload)
        self.assertLess(upload, publish)

    def test_passwordless_secret_uses_explicit_minisign_mode(self):
        self.exercise(password="")


if __name__ == "__main__":
    unittest.main()
