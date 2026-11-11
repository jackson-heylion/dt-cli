"""Compatibility checks preserve independently versioned business Skills."""
import pathlib
import runpy
import unittest

CHECK = runpy.run_path(str(pathlib.Path(__file__).resolve().parents[2] / 'scripts/check_cli_facts.py'))


class CliFactsTest(unittest.TestCase):
    def test_existing_business_skill_accepts_a_compatible_patch_cli(self):
        self.assertTrue(CHECK['compatible_minimum']('0.8.0', '0.8.1'))
        self.assertTrue(CHECK['compatible_minimum']('0.8.1', '0.8.1'))

    def test_retired_root_contract_and_future_minimum_are_rejected(self):
        self.assertFalse(CHECK['compatible_minimum']('0.7.1', '0.8.1'))
        self.assertFalse(CHECK['compatible_minimum']('0.8.2', '0.8.1'))
        self.assertFalse(CHECK['compatible_minimum']('0.8.1', '0.8.0'))

    def test_non_concrete_or_malformed_versions_are_rejected(self):
        for value in ['latest', '0.8', '0.08.1', None]:
            with self.assertRaises(ValueError):
                CHECK['compatible_minimum'](value, '0.8.1')


if __name__ == '__main__':
    unittest.main()
