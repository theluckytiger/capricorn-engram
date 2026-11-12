"""A stand-in for the few Discord API calls her phone bridge makes (`app/src/phone.rs`), to try it without a real bot.

    python tools/fake_discord.py      serves on 127.0.0.1:8765: you are user 42, her bot is 7, your chat is 900

Start her with CAPRI_DISCORD_API=http://127.0.0.1:8765/api/v10 and CAPRI_DISCORD_TOKEN=test-token, and in Settings ›
Phone turn it on with the user ID 42. Then POST /test/say {"text": "..."} is you messaging her, and GET /test/log is
what she posted (her messages, "typing…", pictures), also written as JSON lines to fake_discord.log next to this
file; the last picture she sent is saved as fake_discord_file.png.

Standard library only.
"""

import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

OWNER, BOT, CHANNEL = "42", "7", "900"
HERE = os.path.dirname(os.path.abspath(__file__))
LOG = os.path.join(HERE, "fake_discord.log")
lock = threading.Lock()
messages = [{"id": "1000", "author": {"id": OWNER}, "content": "old message, not news"}]
posted = []
next_id = [1001]


def add(author, text, bot=False):
    with lock:
        m = {"id": str(next_id[0]), "author": {"id": author, **({"bot": True} if bot else {})}, "content": text}
        next_id[0] += 1
        messages.append(m)
        return m


def note(kind, data):
    entry = {"t": time.strftime("%H:%M:%S"), "kind": kind, **data}
    with lock:
        posted.append(entry)
    with open(LOG, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry) + "\n")


class H(BaseHTTPRequestHandler):
    def reply(self, code, body=None):
        data = b"" if body is None else json.dumps(body).encode()
        self.send_response(code)
        if body is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return json.loads(self.rfile.read(n) or b"{}") if n else {}

    def authed(self):
        if self.headers.get("Authorization") != "Bot test-token":
            self.reply(401, {"message": "401: Unauthorized"})
            return False
        return True

    def do_GET(self):
        path, _, query = self.path.partition("?")
        if path == "/test/log":
            with lock:
                return self.reply(200, posted)
        if not self.authed():
            return
        if path == "/api/v10/users/@me":
            return self.reply(200, {"id": BOT, "username": "Capri (test)", "bot": True})
        if path == f"/api/v10/channels/{CHANNEL}/messages":
            q = dict(p.split("=", 1) for p in query.split("&") if "=" in p)
            after, limit = int(q.get("after", "0")), int(q.get("limit", "50"))
            with lock:
                newer = [m for m in messages if int(m["id"]) > after]
            newest_first = sorted(newer, key=lambda m: -int(m["id"]))
            if "after" not in q:
                newest_first = sorted(messages, key=lambda m: -int(m["id"]))
            return self.reply(200, newest_first[:limit])
        self.reply(404, {"message": "Unknown"})

    def do_POST(self):
        if self.path == "/test/say":
            m = add(OWNER, self.body()["text"])
            return self.reply(200, m)
        if not self.authed():
            return
        if self.path == "/api/v10/users/@me/channels":
            b = self.body()
            note("open_dm", {"recipient": b.get("recipient_id")})
            return self.reply(200, {"id": CHANNEL, "type": 1})
        if self.path == f"/api/v10/channels/{CHANNEL}/messages":
            kind = self.headers.get("Content-Type", "")
            if kind.startswith("multipart/form-data"):
                # A message with a file: its text from payload_json, the file saved next to this script.
                raw = self.rfile.read(int(self.headers.get("Content-Length") or 0))
                boundary = kind.split("boundary=", 1)[1].encode()
                text, size = "", 0
                for part in raw.split(b"--" + boundary):
                    head, _, data = part.partition(b"\r\n\r\n")
                    data = data[:-2] if data.endswith(b"\r\n") else data
                    if b'name="payload_json"' in head:
                        text = json.loads(data).get("content", "")
                    elif b'name="files[0]"' in head:
                        size = len(data)
                        with open(os.path.join(HERE, "fake_discord_file.png"), "wb") as f:
                            f.write(data)
                add(BOT, text, bot=True)
                note("file", {"content": text, "bytes": size})
                return self.reply(200, {"id": "x"})
            text = self.body().get("content", "")
            add(BOT, text, bot=True)
            note("message", {"content": text})
            return self.reply(200, {"id": "x"})
        if self.path == f"/api/v10/channels/{CHANNEL}/typing":
            note("typing", {})
            return self.reply(204)
        self.reply(404, {"message": "Unknown"})

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    open(LOG, "w").close()
    ThreadingHTTPServer(("127.0.0.1", 8765), H).serve_forever()
