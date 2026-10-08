"""Portable packages preserve instructions and references across client-specific metadata."""
import hashlib
import json
import pathlib
import runpy
import tempfile
import unittest
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
PACKAGER = runpy.run_path(str(ROOT / 'scripts/release/package_skill.py'))
SOURCE = ROOT / 'skills/dt-cli'


class SkillPackageTest(unittest.TestCase):
    def test_client_variants_preserve_the_same_business_instructions(self):
        with tempfile.TemporaryDirectory() as folder:
            for platform in ['standard', 'qwenwork', 'workbuddy']:
                output = pathlib.Path(folder) / (platform + '.zip')
                PACKAGER['package'](SOURCE, output, platform)
                manifest = json.loads(output.with_suffix('.manifest.json').read_text())
                with zipfile.ZipFile(output) as archive:
                    entry = archive.read('dt-cli/SKILL.md').decode()
                    original = (SOURCE / 'SKILL.md').read_text()
                    self.assertEqual(entry.split('\n---\n', 1)[1], original.split('\n---\n', 1)[1])
                    self.assertIn('dt-cli/.skill-metadata.yaml', archive.namelist())
                    for file in (SOURCE / 'references').glob('*.md'):
                        self.assertEqual(archive.read('dt-cli/references/' + file.name), file.read_bytes())
                    for file in (SOURCE / 'scripts').glob('*'):
                        self.assertEqual(archive.read('dt-cli/scripts/' + file.name), file.read_bytes())
                    if platform == 'standard':
                        self.assertEqual(entry, original)
                        self.assertIn('dt-cli/agents/openai.yaml', archive.namelist())
                    elif platform == 'qwenwork':
                        self.assertIn('name_en:', entry)
                        self.assertIn('argument-hint-zh:', entry)
                        self.assertNotIn('dt-cli/agents/openai.yaml', archive.namelist())
                    else:
                        self.assertIn('agent_created: true', entry)
                        self.assertNotIn('dt-cli/agents/openai.yaml', archive.namelist())
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
