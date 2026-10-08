"""Public export reads a fixed commit, excludes internal records, and preserves existing destinations."""
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SCRIPT = pathlib.Path(__file__).resolve().parents[2] / 'scripts/release/export_public.py'
SPEC = importlib.util.spec_from_file_location('public_export', SCRIPT)
EXPORT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(EXPORT)


class PublicExportTest(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = pathlib.Path(self.temporary.name) / 'source'
        self.root.mkdir()
        self.output = self.root.parent / 'public'
        self.git('init', '-q')
        for path, body in {'src/main.rs': 'committed source\n',
                           'docs/public/README.md': 'public guide\n',
                           'docs/acceptance/internal.md': 'private acceptance\n',
                           '.scratch/task/spec.md': 'internal plan\n'}.items():
            destination = self.root / path
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(body, encoding='utf-8')
        self.git('add', '.')
        self.git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'fixture')
        self.commit = self.git('rev-parse', 'HEAD').strip()

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, text=True)

    def export(self):
        with patch.object(EXPORT, 'ROOT', self.root):
            EXPORT.export(self.commit, self.output)

    def test_export_uses_commit_and_excludes_internal_records_and_history(self):
        (self.root / 'src/main.rs').write_text('uncommitted material\n')
        self.export()
        self.assertEqual((self.output / 'src/main.rs').read_text(), 'committed source\n')
        self.assertEqual((self.output / 'README.md').read_text(), 'public guide\n')
        self.assertFalse((self.output / 'docs').exists())
        self.assertFalse((self.output / '.scratch').exists())
        self.assertFalse((self.output / '.git').exists())
        manifest = json.loads((self.output / 'source-export.json').read_text())
        self.assertEqual(manifest['sourceCommit'], self.commit)
        self.assertEqual(set(manifest['files']), {'README.md', 'src/main.rs'})

    def test_existing_checkout_is_not_overwritten(self):
        self.output.mkdir()
        sentinel = self.output / 'existing.txt'
        sentinel.write_text('preserve')
        with self.assertRaises(SystemExit):
            self.export()
        self.assertEqual(sentinel.read_text(), 'preserve')
        self.assertEqual(list(self.output.iterdir()), [sentinel])


if __name__ == '__main__':
    unittest.main()
