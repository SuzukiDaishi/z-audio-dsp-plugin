#!/usr/bin/env python3
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import json
import mimetypes
from pathlib import Path
import sys


REPO_ROOT = Path(__file__).resolve().parents[2]
WEBCLAP_DIR = REPO_ROOT / "target" / "webclap"
SHELF_ENDPOINT = "/apps/z-audio-wclap-host/__webclap_bundles.json"
SHELF_LABELS = {
    "z-audio-simple-synth": "Synth",
    "z-audio-formula-piano": "Formula Piano",
    "z-audio-vcsl-piano": "VCSL Piano",
    "z-audio-sampler": "Sampler",
    "z-audio-granular": "Granular",
    "z-audio-wavetable": "Wave Synth",
    "z-audio-formula-drums": "Drums",
    "z-audio-simple-eq": "EQ",
    "z-audio-ring-mod": "Ring Mod",
    "z-audio-distortion": "Distortion",
    "z-audio-saturator": "Saturator",
    "z-audio-bitcrusher": "Bitcrusher",
    "z-audio-delay": "Delay",
    "z-audio-chorus": "Chorus",
    "z-audio-flanger": "Flanger",
    "z-audio-phaser": "Phaser",
    "z-audio-tremolo": "Tremolo",
    "z-audio-gate": "Gate",
    "z-audio-diffuser": "Diffuser",
    "z-audio-parametric-reverb": "Reverb",
    "z-audio-limiter": "Limiter",
    "z-audio-compressor": "Compressor",
}
SHELF_ORDER = {name: index for index, name in enumerate(SHELF_LABELS)}
mimetypes.add_type("text/javascript", ".mjs")
mimetypes.add_type("application/wasm", ".wasm")


class HostRequestHandler(SimpleHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?", 1)[0]
        if path == SHELF_ENDPOINT:
            self.send_webclap_shelf()
            return
        super().do_GET()

    def end_headers(self):
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Cross-Origin-Opener-Policy", "same-origin")
        self.send_header("Cross-Origin-Embedder-Policy", "credentialless")
        self.send_header("Cache-Control", "no-store, max-age=0, must-revalidate")
        self.send_header("Pragma", "no-cache")
        self.send_header("Expires", "0")
        super().end_headers()

    def send_webclap_shelf(self):
        bundles = []
        for bundle in sorted(WEBCLAP_DIR.glob("*.wclap.tar.gz"), key=shelf_sort_key):
            plugin_name = bundle.name.removesuffix(".wclap.tar.gz")
            cache_key = bundle.stat().st_mtime_ns
            bundles.append(
                {
                    "label": SHELF_LABELS.get(plugin_name, shelf_label(plugin_name)),
                    "url": f"../../target/webclap/{bundle.name}?v={cache_key}",
                    "file": bundle.name,
                }
            )

        body = json.dumps(bundles, ensure_ascii=False).encode("utf-8")
        self.send_response(200)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def shelf_sort_key(path):
    plugin_name = path.name.removesuffix(".wclap.tar.gz")
    return (SHELF_ORDER.get(plugin_name, len(SHELF_ORDER)), plugin_name)


def shelf_label(plugin_name):
    name = plugin_name.removeprefix("z-audio-")
    return " ".join(part.capitalize() for part in name.split("-"))


def main():
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
    handler = partial(HostRequestHandler, directory=str(REPO_ROOT))
    server = ThreadingHTTPServer(("127.0.0.1", port), handler)
    print(f"Serving Z Audio WebCLAP Host at http://127.0.0.1:{port}/apps/z-audio-wclap-host/")
    print(f"Serving repository root from {REPO_ROOT}")
    server.serve_forever()


if __name__ == "__main__":
    main()
