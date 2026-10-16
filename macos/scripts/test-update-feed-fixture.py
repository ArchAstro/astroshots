#!/usr/bin/env python3
"""Exercise the release verifier with real Sparkle tools and disposable keys."""
import base64
import functools
import http.server
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import zipfile

SCRIPTS = Path(__file__).resolve().parent
TOOLS = subprocess.check_output(
    ['bash', str(SCRIPTS / 'sparkle-tools.sh'), '--download'], text=True
).strip()


def run(command, env):
    return subprocess.run(command, env=env, text=True, capture_output=True)


with tempfile.TemporaryDirectory(prefix='astroshots-feed-fixture-') as tmp:
    root = Path(tmp)
    scripts = root / 'macos/scripts'
    scripts.mkdir(parents=True)
    for name in ['test-update-feed.sh', 'sparkle-tools.sh']:
        shutil.copy2(SCRIPTS / name, scripts / name)
    env = dict(os.environ)
    for key in ['SPARKLE_BIN_DIR', 'SPARKLE_ED_PRIVATE_KEY', 'SPARKLE_ED_KEY_FILE']:
        env.pop(key, None)
    # A random Ed25519 seed in Sparkle's exported format; no Keychain access.
    key = root / 'key'
    key.write_bytes(base64.b64encode(os.urandom(32)))
    key.chmod(0o600)
    env['SPARKLE_ED_KEY_FILE'] = str(key)
    env['HOME'] = str(root / 'empty-home')
    env['PATH'] = str(Path(sys.executable).parent) + os.pathsep + env['PATH']
    public = root / 'public'
    public.mkdir()
    archive = public / 'update.zip'
    with zipfile.ZipFile(archive, 'w') as z:
        z.writestr('fixture.txt', 'original capture')
    signature = subprocess.check_output(
        [str(Path(TOOLS) / 'sign_update'), '--ed-key-file', str(key), '-p', str(archive)],
        text=True,
    ).strip()
    (public / 'notes.html').write_text('Synthetic release notes')

    class QuietHandler(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *_args):
            pass

    server = http.server.ThreadingHTTPServer(
        ('127.0.0.1', 0), functools.partial(QuietHandler, directory=str(public))
    )
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    url = f'http://127.0.0.1:{server.server_port}'

    def feed():
        (public / 'appcast.xml').write_text(f'''<?xml version="1.0"?>
<rss xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><item>
<sparkle:shortVersionString>1.2.3</sparkle:shortVersionString>
<sparkle:version>123</sparkle:version>
<sparkle:releaseNotesLink>{url}/notes.html</sparkle:releaseNotesLink>
<enclosure url="{url}/update.zip" length="{archive.stat().st_size}" sparkle:edSignature="{signature}"/>
</item></channel></rss>''')

    def verify(label, success=True, diagnostic=None):
        result = run(['bash', str(scripts / 'test-update-feed.sh'), url + '/appcast.xml', '1.2.3'], env)
        assert (result.returncode == 0) == success, (label, result.stdout, result.stderr)
        if diagnostic:
            assert diagnostic in result.stdout + result.stderr, (label, result.stdout, result.stderr)
        print('PASS:', label)

    try:
        feed()
        verify('absent optional search directories report actionable error', False, 'Sparkle tools not found')
        fallback = root / 'macos/build/sparkle-tools/bin'
        fallback.parent.mkdir(parents=True)
        fallback.symlink_to(TOOLS, target_is_directory=True)
        verify('fallback directory with absent DerivedData verifies real signature')
        env['SPARKLE_BIN_DIR'] = TOOLS
        verify('explicit shared bin directory verifies real signature')
        env['SPARKLE_BIN_DIR'] = str(root / 'invalid-bin')
        verify('invalid explicit bin fails instead of silently selecting another tool', False, 'SPARKLE_BIN_DIR=')
        env['SPARKLE_BIN_DIR'] = TOOLS
        # Keep ZIP integrity and declared length valid, so only EdDSA can reject.
        with zipfile.ZipFile(archive, 'w') as z:
            z.writestr('fixture.txt', 'tampered capture')
        feed()
        verify('tampered valid ZIP is rejected by real Sparkle signature verification',
               False, 'failed to pass signing verification')
    finally:
        server.shutdown()
        server.server_close()
        thread.join()
