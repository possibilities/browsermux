#!/usr/bin/env python3
"""Small, explicit invariants supplement tests; not a substitute for a security review."""
from pathlib import Path
root=Path(__file__).resolve().parent.parent
engine=(root/'macos/CEF/BrowserEngine.mm').read_text()
helper=(root/'macos/CEF/HelperMain.cc').read_text()
for forbidden in ['no_sandbox=true','no_sandbox = true','ignore-certificate-errors','disable-web-security','remote-debugging-port','GetGlobalContext(']:
    assert forbidden not in engine,forbidden
assert helper.index('sandbox.Initialize')<helper.index('library.LoadInHelper')
assert 'command_line_args_disabled=true' in engine
assert 'OnCertificateError' in engine and 'CEF_PERMISSION_RESULT_DENY' in engine
assert 'contextPaths[key]!=requested' in engine
assert 'CloseBrowser(false)' in engine
assert 'SPBShutdown' in engine and 'clients.empty()' in engine
assert not list(root.glob('**/*.pem'))
print('PASS: source-level sandbox, profile, permission and engine-lifetime safeguards')
