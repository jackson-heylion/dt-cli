"""Bootstrap argument/hash boundaries with a recording process; this is not Windows native acceptance."""
import hashlib
import json
import os
import pathlib
import platform
import shutil
import subprocess
import tempfile
import unittest
import zipfile

ROOT=pathlib.Path(__file__).resolve().parents[2]
PWSH=os.environ.get('DT_CLI_TEST_PWSH') or shutil.which('pwsh')
class BootstrapTest(unittest.TestCase):
    def fixture(self, folder, windows=False):
        binary=folder/('dt-cli.exe' if windows else 'dt-cli')
        binary.write_text('#!/usr/bin/env python3\nimport json,sys\nprint(json.dumps(sys.argv[1:],ensure_ascii=False))\n',encoding='utf-8');binary.chmod(0o755)
        archive=folder/'程序 包.zip'
        with zipfile.ZipFile(archive,'w') as output:
            output.writestr(binary.name,binary.read_bytes());output.writestr('manifest.json',json.dumps({'sha256':hashlib.sha256(binary.read_bytes()).hexdigest()}))
        script=folder/('install.ps1' if windows else 'install.sh');shutil.copy2(ROOT/'scripts/release'/script.name,script)
        return script,archive,hashlib.sha256(archive.read_bytes()).hexdigest()
    @unittest.skipUnless(platform.system()=='Darwin','macOS bootstrap tools')
    def test_shell_bootstrap_preserves_arguments_and_checks_extracted_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);script,archive,digest=self.fixture(root)
            argv=['--package',str(archive),'--sha256',digest,'--directory','中文 空格 "引号"\n换行 > literal $(false)']
            output=subprocess.run(['sh',str(script),*argv],capture_output=True,text=True)
            self.assertEqual(output.returncode,0,output.stderr);self.assertEqual(json.loads(output.stdout),['install',*argv])
            (root/'dt-cli').write_text('changed')
            rejected=subprocess.run(['sh',str(script),*argv],capture_output=True,text=True)
            self.assertNotEqual(rejected.returncode,0);self.assertEqual(rejected.stdout,'')
    @unittest.skipUnless(platform.system()=='Darwin','macOS bootstrap tools')
    def test_bad_zip_hash_stops_before_the_recording_process(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);script,archive,_=self.fixture(root)
            output=subprocess.run(['sh',str(script),'--package',str(archive),'--sha256','0'*64,'--candidate'],capture_output=True,text=True)
            self.assertNotEqual(output.returncode,0);self.assertEqual(output.stdout,'')
    @unittest.skipUnless(PWSH,'PowerShell runtime not supplied')
    def test_powershell_bootstrap_preserves_native_argument_values(self):
        if platform.system()=='Windows':self.skipTest('Recording process fixture is POSIX; Windows native release uses final EXE')
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);script,archive,digest=self.fixture(root,True)
            argv=['--package',str(archive),'--sha256',digest,'--directory','中文 空格 "引号"\n换行 > & literal \\last\\']
            output=subprocess.run([PWSH,'-NoLogo','-NoProfile','-File',str(script),*argv],capture_output=True,text=True)
            self.assertEqual(output.returncode,0,output.stderr);self.assertEqual(json.loads(output.stdout),['install',*argv])
if __name__=='__main__':unittest.main()
