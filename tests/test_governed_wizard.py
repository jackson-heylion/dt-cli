"""Exercise the actual wizard helpers without network, secrets, or deployment actions."""
import subprocess
import json
import tempfile
import unittest
from pathlib import Path

SCRIPT = (Path(__file__).resolve().parents[1] / "scripts/governed-stg-acceptance.sh").read_text()
HELPERS = SCRIPT[SCRIPT.index("# ask_or KEY"):SCRIPT.index('banner "受控访问票 01')]


class WizardSafety(unittest.TestCase):
    def test_new_run_archives_old_passes(self):
        start = SCRIPT.index("# Each run gets fresh evidence.")
        source = SCRIPT[start:].split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            evidence = Path(directory) / "evidence.json"
            evidence.write_text('{"batch":{"complete":true}}')
            subprocess.run(["python3", "-", str(evidence)], input=source,
                           text=True, check=True, timeout=5)
            self.assertEqual(json.loads(evidence.read_text()), {})
            archives = list(Path(directory).glob("evidence-*.json"))
            self.assertEqual(len(archives), 1)
            self.assertTrue(json.loads(archives[0].read_text())["batch"]["complete"])

    def test_missing_clipboard_never_prints_secret(self):
        shell = "set -euo pipefail\nwarn() { printf '%s\\n' \"$1\"; }\nnote() { :; }\n"
        shell += HELPERS
        shell += '''
to_clipboard() { return 1; }
hand_over_config AUTH 'auth:
  cli-api:
    hmac-secret: "synthetic-secret-must-never-appear"'
'''
        result = subprocess.run(["bash", "-c", shell], capture_output=True, text=True, timeout=5)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("synthetic-secret-must-never-appear", result.stdout + result.stderr)
        self.assertIn("没有可用的剪贴板工具", result.stdout)

    def test_secret_clipboard_is_tracked_and_cleared(self):
        shell = "set -euo pipefail\nwarn() { :; }\nnote() { :; }\n"
        shell += HELPERS
        shell += '''
SECRET_CLIPBOARD=false
to_clipboard() { copied="$1"; }
hand_over_config AUTH 'hmac-secret: "synthetic-secret"'
[[ "$SECRET_CLIPBOARD" == true && "$copied" == *synthetic-secret* ]]
clear_clipboard
[[ "$SECRET_CLIPBOARD" == false && -z "$copied" ]]
'''
        result = subprocess.run(["bash", "-c", shell], capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("synthetic-secret", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
