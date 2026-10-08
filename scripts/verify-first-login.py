#!/usr/bin/env python3
"""Runs only this ticket's native-process and isolated IAM tests. Never starts IamApplication."""
import argparse
import json
import pathlib
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--iam', type=pathlib.Path, required=True, help='dt-iam checkout containing the companion implementation')
args = parser.parse_args()
root = pathlib.Path(__file__).resolve().parents[1]
iam = args.iam.resolve()
if not (iam / 'cli/src/test/java/com/datousoft/iam/cli/CliHttpTest.java').is_file():
    parser.error('IAM checkout does not contain CliHttpTest')

def run(argv, cwd=root):
    subprocess.run(argv, cwd=cwd, check=True)

run(['cargo', 'fmt', '--check'])
run(['cargo', 'clippy', '--locked', '--all-targets', '--', '-D', 'warnings'])
run(['cargo', 'test', '--locked'])
artifacts = subprocess.run(['cargo', 'test', '--locked', '--test', 'iam_process', '--no-run', '--message-format=json'], cwd=root, text=True, stdout=subprocess.PIPE, check=True)
executables = [item['executable'] for line in artifacts.stdout.splitlines() if (item := json.loads(line)).get('reason') == 'compiler-artifact' and item.get('executable') and item['target']['name'] == 'iam_process']
if len(executables) != 1:
    raise SystemExit('Expected exactly one native IAM process test executable')
run(['mvn', '-q', '-pl', 'cli', '-am', 'clean', 'test', '-Dmaven.gitcommitid.skip=true', '-Dtest=CliHttpTest,CliPropertiesTest,CliPasswordIdentityServiceTest', '-Dsurefire.failIfNoSpecifiedTests=false', '-Dspring.cloud.bootstrap.enabled=false', '-Ddt.cli.process-test=' + executables[0]], iam)
run(['cargo', 'run', '--locked', '--quiet', '--', 'version'])
