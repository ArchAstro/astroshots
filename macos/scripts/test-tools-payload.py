#!/usr/bin/env python3
import importlib.util
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

spec = importlib.util.spec_from_file_location('payload', Path(__file__).with_name('tools-payload.py'))
payload = importlib.util.module_from_spec(spec)
spec.loader.exec_module(payload)
with tempfile.TemporaryDirectory(prefix='tools-reject-') as tmp:
    root = Path(tmp) / 'Tools'
    shutil.copytree(sys.argv[1], root)
    def reject(name, mutate, restore, expected):
        mutate()
        try:
            payload.validate(root)
        except ValueError as error:
            assert expected in str(error), str(error)
            print(f'PASS: {name}')
        else:
            raise AssertionError(f'accepted {name}')
        finally:
            restore()
    link = root / 'escape'
    reject('reject symlink', lambda: link.symlink_to('/tmp'), lambda: link.unlink(), 'symlink')
    cli = root / 'cli/node_modules/astroshot/bin/astroshot.mjs'
    original = cli.read_bytes()
    reject('reject missing file', lambda: cli.unlink(), lambda: cli.write_bytes(original), 'missing file')
    # Restore original mode as well after replacing the file.
    cli.chmod(Path(sys.argv[1], 'cli/node_modules/astroshot/bin/astroshot.mjs').stat().st_mode)
    reject('reject integrity mismatch', lambda: cli.write_bytes(original + b'\n'), lambda: cli.write_bytes(original), 'integrity')
    launcher = root / 'bin/astroshot'
    reject('reject network launcher', lambda: launcher.write_text('#!/bin/sh\nnpx astroshot "$@"\n'), lambda: launcher.write_text(payload.LAUNCHER), 'launcher')
    native = root / 'host-library'
    source = Path(tmp) / 'host.c'
    source.write_text('int main(void) { return 0; }\n')
    def host_binary():
        subprocess.run(['clang', str(source), '-o', str(native)], check=True)
        subprocess.run(['install_name_tool', '-change', '/usr/lib/libSystem.B.dylib', '/opt/homebrew/lib/libSystem.B.dylib', str(native)], check=True)
    reject('reject host dylib', host_binary, lambda: native.unlink(missing_ok=True), 'non-system dylib')
    def escaping_dylib():
        subprocess.run(['clang', str(source), '-o', str(native)], check=True)
        subprocess.run(
            ['install_name_tool', '-change', '/usr/lib/libSystem.B.dylib',
             '@loader_path/../../../../../../tmp/libSystem.B.dylib', str(native)],
            check=True,
        )
    reject('reject escaping dylib', escaping_dylib, lambda: native.unlink(missing_ok=True), 'escaping dylib')
    def prefix_escape():
        subprocess.run(['clang', str(source), '-o', str(native)], check=True)
        subprocess.run(
            ['install_name_tool', '-change', '/usr/lib/libSystem.B.dylib',
             '/usr/lib/../../../../tmp/libSystem.B.dylib', str(native)],
            check=True,
        )
    reject('reject prefix-escape dylib', prefix_escape, lambda: native.unlink(missing_ok=True), 'escaping dylib')
    src_arch = subprocess.check_output(
        ['/usr/bin/lipo', '-archs', str(Path(sys.argv[1]) / 'node/bin/node')], text=True
    ).split()[0]
    foreign = 'x86_64' if src_arch == 'arm64' else 'arm64'
    def wrong_architecture():
        subprocess.run(['clang', '-arch', foreign, str(source), '-o', str(native)], check=True)
    reject('reject wrong architecture', wrong_architecture, lambda: native.unlink(missing_ok=True), 'wrong architecture')
    payload.validate(root)
