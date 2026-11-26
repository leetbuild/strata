#!/usr/bin/env python3
"""A local stand-in for the Nexus API and CDN, for the window's download test.

usage: fake-nexus.py <archive> <port file>

Serves one file as skyrimspecialedition mod 5, file 77 ("Fake Mod"): files/77, the mod's
info, download_link (pointing back here) and /cdn/<name> with Range support, sent slowly
(64 KiB every 10 ms) so a pause lands mid-way. Writes the port it listens on to <port file>.
Runs until killed. Same replies as strata_nexus::fake (the Rust tests' server).
"""
import json
import os
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote, urlparse

archive = sys.argv[1]
name = os.path.basename(archive)
data = open(archive, "rb").read()
API = "/v1/games/skyrimspecialedition/mods/5"


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        path = unquote(urlparse(self.path).path)
        base = f"http://127.0.0.1:{self.server.server_port}"
        replies = {
            f"{API}/files/77": {"file_id": 77, "name": "Fake Mod Main", "version": "1.0", "category_id": 1,
                                "file_name": name, "description": "d"},
            API: {"mod_id": 5, "name": "Fake Mod", "version": "1.0", "category_id": 42},
            f"{API}/files/77/download_link": [{"name": "CDN", "short_name": "CDN", "URI": f"{base}/cdn/{name}"}],
            "/v1/users/validate": {"user_id": 1, "name": "tester"},
        }
        if path in replies:
            body = json.dumps(replies[path]).encode()
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif path == f"/cdn/{name}":
            start = 0
            rng = self.headers.get("Range")
            if rng and rng.startswith("bytes="):
                start = int(rng[6:].rstrip("-"))
            self.send_response(206 if rng else 200)
            self.send_header("Content-Length", str(len(data) - start))
            self.end_headers()
            try:
                for i in range(start, len(data), 65536):
                    self.wfile.write(data[i:i + 65536])
                    time.sleep(0.01)
            except (BrokenPipeError, ConnectionResetError):
                pass
        else:
            self.send_response(404)
            self.end_headers()


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
with open(sys.argv[2], "w") as f:
    f.write(str(server.server_port))
server.serve_forever()
