"""Tests for bin/g7-share against a local stand-in for Dexcom Share.

The helper is imported as a module and pointed at a temporary home directory
and a loopback server, so nothing here touches the real ~/.config or Dexcom.
"""

import importlib.machinery
import importlib.util
import io
import json
import os
import stat
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
HELPER = ROOT / "bin" / "g7-share"

APP_ID = "d89443d2-327c-4a6f-89e5-496bbb0317db"
ACCOUNT_ID = "11111111-2222-3333-4444-555555555555"
NULL_UUID = "00000000-0000-0000-0000-000000000000"


def load_helper():
    loader = importlib.machinery.SourceFileLoader("g7_share", str(HELPER))
    spec = importlib.util.spec_from_loader("g7_share", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


class FakeShare:
    """State for the stand-in server. Tests poke it between calls."""

    def __init__(self):
        self.accounts = {"kid@example.com": ("correct horse", ACCOUNT_ID)}
        self.sessions = set()
        self.calls = []
        self.lockout = False
        self.kill_sessions = False
        self.huge = False
        self.redirect = False
        self.readings = [
            {"WT": "Date(1760112345000)", "ST": "Date(1760112345000)", "DT": "Date(1760112345000-0500)",
             "Value": 142, "Trend": "Flat"},
            {"WT": "Date(1760112045000)", "ST": "Date(1760112045000)", "DT": "Date(1760112045000-0500)",
             "Value": 138, "Trend": 3},
            {"WT": "/Date(1760111745000+0000)/", "Value": 131, "Trend": "SingleUp"},
            {"WT": "garbage", "Value": 100, "Trend": "Flat"},
            {"WT": "Date(1760111445000)", "Value": "high", "Trend": "Flat"},
            {"WT": "Date(1760111145000)", "Value": 999, "Trend": "Flat"},
            {"WT": "Date(1760110845000)", "Value": 120, "Trend": "Sideways"},
        ]


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def _json(self, status, doc):
        raw = json.dumps(doc).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def _fail(self, code):
        self._json(500, {"Code": code, "Message": "nope", "SubCode": "", "TypeName": "FaultException"})

    def do_POST(self):
        share = self.server.share
        length = int(self.headers.get("Content-Length") or 0)
        body = json.loads(self.rfile.read(length) or b"null") if length else None
        path, _, query = self.path.partition("?")
        share.calls.append(path)
        if share.redirect:
            self.send_response(302)
            self.send_header("Location", "http://127.0.0.1:1/elsewhere")
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if path.endswith("/General/AuthenticatePublisherAccount"):
            if share.lockout:
                return self._fail("SSO_AuthenticateMaxAttemptsExceeed")
            if body.get("applicationId") != APP_ID:
                return self._fail("InvalidArgument")
            entry = share.accounts.get(body.get("accountName"))
            if entry is None:
                return self._fail("SSO_AuthenticateAccountNotFound")
            if entry[0] != body.get("password"):
                return self._fail("SSO_AuthenticatePasswordInvalid")
            return self._json(200, entry[1])
        if path.endswith("/General/LoginPublisherAccountById"):
            for password, account_id in share.accounts.values():
                if account_id == body.get("accountId"):
                    if password != body.get("password"):
                        return self._fail("AccountPasswordInvalid")
                    sid = "aaaaaaaa-bbbb-cccc-dddd-%012d" % len(share.sessions)
                    share.sessions.add(sid)
                    return self._json(200, sid)
            return self._json(200, NULL_UUID)
        if path.endswith("/Publisher/ReadPublisherLatestGlucoseValues"):
            params = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
            sid = params.get("sessionId")
            if share.kill_sessions:
                share.sessions.clear()
                share.kill_sessions = False
            if sid not in share.sessions:
                return self._fail("SessionNotValid")
            if share.huge:
                raw = b"[" + b",".join([b'{"WT":"Date(1760112345000)","Value":100,"Trend":"Flat"}'] * 9000) + b"]"
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)
                return
            count = int(params.get("maxCount", "1"))
            return self._json(200, share.readings[:count])
        self._fail("Unknown")


class HelperTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        cls.server.share = FakeShare()
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base = "http://127.0.0.1:%d/Services" % cls.server.server_address[1]

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        self.share = FakeShare()
        self.server.share = self.share
        self.tmp = tempfile.TemporaryDirectory()
        self.mod = load_helper()
        self.mod._home = lambda: self.tmp.name
        self.point_at(self.base)

    def tearDown(self):
        self.tmp.cleanup()

    def point_at(self, base):
        for region in ("us", "ous", "jp"):
            self.mod.BASES[region] = base

    # -- helpers

    def run_cmd(self, *args, stdin=None):
        out = io.StringIO()
        old_out, old_in = sys.stdout, sys.stdin
        sys.stdout = out
        if stdin is not None:
            sys.stdin = io.StringIO(stdin)
        try:
            rc = self.mod.main(["g7-share", *args])
        finally:
            sys.stdout, sys.stdin = old_out, old_in
        text = out.getvalue().strip()
        return rc, (json.loads(text) if text else None)

    def sign_in(self, **overrides):
        doc = {"region": "us", "account_name": "kid@example.com", "password": "correct horse"}
        doc.update(overrides)
        return self.run_cmd("login", "--stdin", stdin=json.dumps(doc))

    def path(self, *parts):
        return Path(self.tmp.name, *parts)

    def mode(self, p):
        return stat.S_IMODE(os.lstat(p).st_mode)

    # -- tests

    def test_login_stores_private_files_and_probes(self):
        rc, doc = self.sign_in()
        self.assertEqual(rc, 0, doc)
        self.assertTrue(doc["ok"])
        self.assertEqual(doc["account"], "k***@example.com")
        self.assertEqual(doc["latest"]["v"], 142)
        cred = self.path(".config", "g7-companion", "credentials.json")
        self.assertEqual(self.mode(cred), 0o600)
        self.assertEqual(self.mode(cred.parent), 0o700)
        self.assertEqual(self.mode(self.path(".local", "state", "g7-companion")), 0o700)
        self.assertTrue(self.path(".local", "state", "g7-companion", "session.json").exists())
        self.assertEqual(json.loads(cred.read_text())["password"], "correct horse")

    def test_login_rejects_wrong_password_without_storing(self):
        rc, doc = self.sign_in(password="wrong")
        self.assertEqual(rc, 1)
        self.assertFalse(doc["ok"])
        self.assertIn("password", doc["message"].lower())
        self.assertFalse(self.path(".config", "g7-companion", "credentials.json").exists())

    def test_login_with_account_id_skips_authenticate(self):
        rc, doc = self.sign_in(account_name="", account_id=ACCOUNT_ID)
        self.assertEqual(rc, 0, doc)
        self.assertNotIn("/Services/General/AuthenticatePublisherAccount", self.share.calls)
        self.assertEqual(doc["account"], "account ID")

    def test_read_normalizes_and_drops_bad_entries(self):
        self.sign_in()
        rc, doc = self.run_cmd("read", "--minutes", "180", "--max-count", "36")
        self.assertEqual(rc, 0)
        self.assertTrue(doc["ok"])
        self.assertEqual(doc["source"], "live")
        values = [(r["t"], r["v"], r["trend"]) for r in doc["readings"]]
        self.assertEqual(values, [
            (1760112345, 142, "Flat"),
            (1760112045, 138, "FortyFiveUp"),
            (1760111745, 131, "SingleUp"),
            (1760110845, 120, "None"),
        ])

    def test_read_reuses_session_and_recovers_once_when_it_dies(self):
        self.sign_in()
        self.share.calls.clear()
        self.run_cmd("read")
        self.assertEqual(self.share.calls, ["/Services/Publisher/ReadPublisherLatestGlucoseValues"])
        self.share.kill_sessions = True
        self.share.calls.clear()
        rc, doc = self.run_cmd("read")
        self.assertTrue(doc["ok"], doc)
        self.assertEqual(self.share.calls, [
            "/Services/Publisher/ReadPublisherLatestGlucoseValues",
            "/Services/General/AuthenticatePublisherAccount",
            "/Services/General/LoginPublisherAccountById",
            "/Services/Publisher/ReadPublisherLatestGlucoseValues",
        ])

    def test_password_change_stops_all_further_authentication(self):
        self.sign_in()
        self.share.accounts["kid@example.com"] = ("changed", ACCOUNT_ID)
        self.share.kill_sessions = True
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertTrue(doc["needs_login"])
        self.assertEqual(doc["source"], "cache")
        self.assertEqual(doc["readings"][0]["v"], 142)
        self.share.calls.clear()
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertTrue(doc["needs_login"])
        self.assertEqual(self.share.calls, [], "must not touch Dexcom again until the user signs in")
        rc, status = self.run_cmd("status")
        self.assertTrue(status["auth_failed"])
        rc, doc = self.sign_in(password="changed")
        self.assertTrue(doc["ok"])
        rc, doc = self.run_cmd("read")
        self.assertTrue(doc["ok"])

    def test_lockout_sets_a_long_backoff(self):
        self.sign_in()
        self.share.lockout = True
        self.share.kill_sessions = True
        before = int(time.time())
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertEqual(doc["error"], "SSO_AuthenticateMaxAttemptsExceeed")
        self.assertGreaterEqual(doc["retry_after"], before + 29 * 60)
        self.share.calls.clear()
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertEqual(self.share.calls, [])
        self.assertEqual(doc["readings"][0]["v"], 142)

    def test_network_failure_returns_cache_and_short_backoff(self):
        self.sign_in()
        self.point_at("http://127.0.0.1:1/Services")
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertEqual(doc["error"], "network")
        self.assertFalse(doc["needs_login"])
        self.assertEqual(doc["source"], "cache")
        self.assertEqual(doc["readings"][0]["v"], 142)
        self.point_at(self.base)
        self.share.calls.clear()
        rc, doc = self.run_cmd("read")
        self.assertEqual(self.share.calls, [], "a transient failure waits before retrying")
        self.assertIn(doc["error"], ("network",))

    def test_oversize_body_is_refused(self):
        self.sign_in()
        self.share.huge = True
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertEqual(doc["error"], "oversize")

    def test_redirects_are_not_followed(self):
        self.sign_in()
        self.share.redirect = True
        self.share.calls.clear()
        rc, doc = self.run_cmd("read")
        self.assertFalse(doc["ok"])
        self.assertEqual(doc["error"], "http_302")
        self.assertEqual(len(self.share.calls), 1)

    def test_logout_removes_everything(self):
        self.sign_in()
        rc, doc = self.run_cmd("logout")
        self.assertTrue(doc["ok"])
        self.assertFalse(self.path(".config", "g7-companion", "credentials.json").exists())
        self.assertEqual(list(self.path(".local", "state", "g7-companion").iterdir()), [])
        rc, doc = self.run_cmd("read")
        self.assertEqual(doc["error"], "signed_out")
        self.assertTrue(doc["needs_login"])

    def test_planted_symlink_is_removed_not_followed(self):
        victim = self.path("victim.json")
        victim.write_text('{"version":1,"region":"us","account_name":"x","password":"y"}')
        cfg = self.path(".config", "g7-companion")
        cfg.mkdir(parents=True)
        os.symlink(victim, cfg / "credentials.json")
        rc, doc = self.run_cmd("status")
        self.assertFalse(doc["signed_in"])
        self.assertFalse(os.path.lexists(cfg / "credentials.json"))
        self.assertTrue(victim.exists())
        self.assertIn("password", victim.read_text())

    def test_world_readable_credentials_are_tightened(self):
        self.sign_in()
        cred = self.path(".config", "g7-companion", "credentials.json")
        os.chmod(cred, 0o644)
        self.run_cmd("status")
        self.assertEqual(self.mode(cred), 0o600)

    def test_origins_are_fixed_dexcom_hosts(self):
        fresh = load_helper()
        self.assertEqual(fresh._base_url("us"), "https://share2.dexcom.com/ShareWebServices/Services")
        self.assertEqual(fresh._base_url("ous"), "https://shareous1.dexcom.com/ShareWebServices/Services")
        self.assertEqual(fresh._base_url("jp"), "https://share.dexcom.jp/ShareWebServices/Services")
        os.environ["G7_SHARE_BASE_URL"] = "http://127.0.0.1:1/"
        try:
            self.assertEqual(load_helper()._base_url("us"), "https://share2.dexcom.com/ShareWebServices/Services")
        finally:
            os.environ.pop("G7_SHARE_BASE_URL", None)

    def test_bad_stdin_login_input(self):
        rc, doc = self.run_cmd("login", "--stdin", stdin="not json")
        self.assertEqual(rc, 2)
        rc, doc = self.run_cmd("login", "--stdin", stdin=json.dumps({"region": "mars", "account_name": "a", "password": "b"}))
        self.assertEqual(rc, 2)
        rc, doc = self.run_cmd("login", "--stdin", stdin=json.dumps({"region": "us", "account_id": "nope", "password": "b"}))
        self.assertEqual(rc, 2)
        rc, doc = self.run_cmd("login", "--stdin", stdin=json.dumps({"region": "us", "account_name": "a", "password": "b\nc"}))
        self.assertEqual(rc, 2)

    def test_help_runs_isolated(self):
        proc = subprocess.run([sys.executable, "-I", "-S", str(HELPER), "--help"], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 2)
        self.assertIn("g7-share login", proc.stderr)


if __name__ == "__main__":
    unittest.main()
