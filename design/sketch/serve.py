"""Serves the repo root so the sketch can use the app's own icons, and tells the page when a
sketch file changed so it reloads by itself (CSS changes swap the stylesheet in place).

    python -I design/sketch/serve.py        ->  http://localhost:5180/design/sketch/
"""

import http.server
import json
import os
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
SKETCH = pathlib.Path(__file__).resolve().parent
PORT = int(os.environ.get("PORT", "5180"))


def stamps():
    css, other = 0.0, 0.0
    for path in SKETCH.iterdir():
        if path.suffix == ".css":
            css = max(css, path.stat().st_mtime)
        elif path.suffix in (".html", ".js"):
            other = max(other, path.stat().st_mtime)
    return {"css": css, "other": other}


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(ROOT), **kwargs)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()

    def do_GET(self):
        if self.path.startswith("/__stamps"):
            body = json.dumps(stamps()).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        super().do_GET()

    def log_message(self, *args):
        pass


if __name__ == "__main__":
    print(f"http://localhost:{PORT}/design/sketch/", flush=True)
    http.server.ThreadingHTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
