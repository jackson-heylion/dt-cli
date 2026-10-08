"""One universal archive preserves business instructions and all host metadata."""
import hashlib
import json
import pathlib
import runpy
import tempfile
import unittest
import zipfile
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
PACKAGER = runpy.run_path(str(ROOT / 'scripts/release/package_skill.py'))
SOURCE = ROOT / 'skills/dt-cli'


class SkillPackageTest(unittest.TestCase):
    def test_skill_zip_does_not_depend_on_packaging_host_os(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            first = root / 'unix.zip'
            second = root / 'windows.zip'
            PACKAGER['package'](SOURCE, first)
            original = zipfile.ZipInfo

            class WindowsInfo(original):
                def __init__(self, *args, **kwargs):
                    super().__init__(*args, **kwargs)
                    self.create_system = 0

            with patch.object(zipfile, 'ZipInfo', WindowsInfo):
                PACKAGER['package'](SOURCE, second)
            self.assertEqual(first.read_bytes(), second.read_bytes())

    def test_universal_package_preserves_instructions_and_all_host_metadata(self):
        with tempfile.TemporaryDirectory() as folder:
            output = pathlib.Path(folder) / 'dt-cli-skill.zip'
            PACKAGER['package'](SOURCE, output)
            manifest = json.loads(output.with_suffix('.manifest.json').read_text())
            self.assertEqual(manifest['platform'], 'universal')
            with zipfile.ZipFile(output) as archive:
                entry = archive.read('dt-cli/SKILL.md').decode()
                original = (SOURCE / 'SKILL.md').read_text()
                self.assertEqual(entry.split('\n---\n', 1)[1], original.split('\n---\n', 1)[1])
                self.assertIn('dt-cli/.skill-metadata.yaml', archive.namelist())
                self.assertIn('dt-cli/agents/openai.yaml', archive.namelist())
                self.assertIn('name_en:', entry)
                self.assertIn('description_zh:', entry)
                self.assertIn('argument-hint-zh:', entry)
                self.assertIn('user-invocable: true', entry)
                self.assertIn('agent_created: true', entry)
                for file in (SOURCE / 'references').glob('*.md'):
                    self.assertEqual(archive.read('dt-cli/references/' + file.name), file.read_bytes())
                for file in (SOURCE / 'scripts').glob('*'):
                    self.assertEqual(archive.read('dt-cli/scripts/' + file.name), file.read_bytes())
                for path, digest in manifest['files'].items():
                    self.assertEqual(hashlib.sha256(archive.read(path)).hexdigest(), digest)
            self.assertEqual(hashlib.sha256(output.read_bytes()).hexdigest(), manifest['sha256'])

    def test_missing_source_file_keeps_existing_output(self):
        with tempfile.TemporaryDirectory() as folder:
            root = pathlib.Path(folder)
            source = root / 'test-skill'; source.mkdir()
            (source / 'SKILL.md').write_text('---\nname: test-skill\ndescription: Test\n---\nTest')
            output = root / 'existing.zip'; output.write_bytes(b'prior-package')
            with self.assertRaises(SystemExit):
                PACKAGER['package'](source, output)
            self.assertEqual(output.read_bytes(), b'prior-package')


if __name__ == '__main__':
    unittest.main()
