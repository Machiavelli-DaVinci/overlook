#!/usr/bin/env python3
"""Run Overlook's independent CalDAV qualification with disposable Radicale storage.

Use a venv with radicale==3.8.3 installed. Requires Cargo's cached dependencies.
No real account, keyring, installed application or external service is used.
"""
import base64
import http.client
import importlib.metadata
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ROOT / "crates/overlook-app/tests/fixtures"
OWNER = "/fixture@example.test"
AUTH = "Basic " + base64.b64encode(b"fixture@example.test:fixture-password").decode()


def run():
    version = importlib.metadata.version("radicale")
    if version != "3.8.3":
        raise RuntimeError(f"Expected Radicale 3.8.3; found {version}")
    with tempfile.TemporaryDirectory(prefix="overlook-reference-") as temporary:
        folder = Path(temporary)
        cert = folder / "certificate.pem"
        cert.write_text(ssl.DER_cert_to_PEM_cert((FIXTURES / "resource-test-cert.der").read_bytes()))
        key = folder / "key.pem"
        key.write_text("-----BEGIN PRIVATE KEY-----\n" + base64.encodebytes((FIXTURES / "resource-test-key.der").read_bytes()).decode() + "-----END PRIVATE KEY-----\n")
        users = folder / "users"
        users.write_text("fixture@example.test:fixture-password\n")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        config = folder / "config"
        config.write_text(f"""[server]
hosts = 127.0.0.1:{port}
ssl = True
certificate = {cert}
key = {key}
[auth]
type = htpasswd
htpasswd_filename = {users}
htpasswd_encryption = plain
[storage]
filesystem_folder = {folder / 'storage'}
[logging]
level = warning
""")
        context = ssl.create_default_context(cafile=str(cert))

        class Connection(http.client.HTTPSConnection):
            def connect(self):
                sock = socket.create_connection(("127.0.0.1", port), timeout=3)
                self.sock = context.wrap_socket(sock, server_hostname="synthetic.example")

        def request(method, path, body=""):
            client = Connection("synthetic.example", timeout=3)
            try:
                client.request(method, path, body.encode(), headers={"Authorization": AUTH, "Content-Type": "application/xml" if method == "MKCALENDAR" else "text/calendar"})
                response = client.getresponse()
                data = response.read().decode()
                if response.status not in (200, 201, 204, 207):
                    raise RuntimeError(f"{method} {path}: HTTP {response.status}: {data}")
            finally:
                client.close()

        with (folder / "server.log").open("w+") as log:
            server = subprocess.Popen([sys.executable, "-m", "radicale", "--config", str(config)], stdout=log, stderr=log)
            try:
                for _ in range(100):
                    if server.poll() is not None:
                        raise RuntimeError("Radicale stopped during startup")
                    try:
                        request("OPTIONS", "/")
                        break
                    except (OSError, http.client.HTTPException):
                        time.sleep(0.05)
                else:
                    raise RuntimeError("Radicale did not start")
                for name in ("personal", "empty"):
                    request("MKCALENDAR", f"{OWNER}/{name}/", f'<c:mkcalendar xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:d="DAV:"><d:set><d:prop><d:displayname>{name}</d:displayname><c:supported-calendar-component-set><c:comp name="VEVENT"/></c:supported-calendar-component-set></d:prop></d:set></c:mkcalendar>')
                def put(name, event):
                    timezone = ""
                    if "TZID=Europe/Berlin" in event:
                        # A self-contained TZID resource includes its VTIMEZONE.
                        timezone = "BEGIN:VTIMEZONE\nTZID:Europe/Berlin\nBEGIN:STANDARD\nDTSTART:20001029T030000\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=10\nTZOFFSETFROM:+0200\nTZOFFSETTO:+0100\nEND:STANDARD\nBEGIN:DAYLIGHT\nDTSTART:20000326T020000\nRRULE:FREQ=YEARLY;BYDAY=-1SU;BYMONTH=3\nTZOFFSETFROM:+0100\nTZOFFSETTO:+0200\nEND:DAYLIGHT\nEND:VTIMEZONE\n"
                    body = "BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//Independent qualification//EN\n" + timezone + "BEGIN:VEVENT\nDTSTAMP:20261001T000000Z\n" + event + "\nEND:VEVENT\nEND:VCALENDAR\n"
                    request("PUT", f"{OWNER}/personal/{name}.ics", body.replace("\n", "\r\n"))
                put("single", "UID:reference-single\nDTSTART:20261024T160000Z\nDTEND:20261024T170000Z\nSUMMARY:Reference single\nLOCATION:Reference room\nX-QUALIFICATION:preserve-me")
                put("series", "UID:reference-series\nDTSTART;TZID=Europe/Berlin:20261024T090000\nDTEND;TZID=Europe/Berlin:20261024T100000\nRRULE:FREQ=DAILY;COUNT=3\nSUMMARY:Reference recurring")
                put("allday", "UID:reference-allday\nDTSTART;VALUE=DATE:20261024\nDTEND;VALUE=DATE:20261027\nSUMMARY:Reference all day")
                env = dict(os.environ, OVERLOOK_REFERENCE_CALDAV_ADDR=f"127.0.0.1:{port}")
                print(f"Testing Radicale {version} over verified loopback TLS", flush=True)
                result = subprocess.run(["cargo", "test", "-p", "overlook", "--offline", "reference_server_round_trip", "--", "--ignored", "--nocapture"], cwd=ROOT, env=env)
                if result.returncode:
                    raise RuntimeError("Reference qualification failed")
            except BaseException:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    run()
