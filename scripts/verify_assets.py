#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Checks that need an independent implementation: decode the generated PNG with Pillow and
run the theme injection script in a real Chromium (Playwright).

    python3 scripts/verify_assets.py

Needs: cargo, Pillow, playwright (with its Chromium).
"""
import io
import subprocess
import sys

from PIL import Image
from playwright.sync_api import sync_playwright

DUMP = ["cargo", "run", "-q", "-p", "rustcord-core", "--example", "dump", "--"]


def run(args, stdin=b""):
    return subprocess.run(DUMP + args, input=stdin, capture_output=True, check=True).stdout


def check_png():
    for size in (64, 128, 256):
        img = Image.open(io.BytesIO(run(["png", str(size)])))
        img.load()
        assert img.size == (size, size) and img.mode == "RGBA", (img.size, img.mode)
        assert img.getpixel((size // 2, size // 2)) == (0x58, 0x65, 0xF2, 255)
        assert img.getpixel((0, 0))[3] == 0
    print("png: decodes in Pillow at 64/128/256, centre colour and transparent corner correct")


def check_theme_script():
    css = (
        ":root{--rustcord-accent:#5865f2;}\n"
        "body{background:rgb(1, 2, 3) !important}\n"
        '.x::after{content:"quote \\" and </style><script>window.pwned=1</script>"}\n'
    )
    script = run(["theme-script"], css.encode()).decode()
    pages = {
        "no CSP": "<!doctype html><title>t</title><body><p id=x>x</p>",
        "CSP like a site that allows inline styles": (
            "<!doctype html><meta http-equiv=Content-Security-Policy "
            "content=\"style-src 'self' 'unsafe-inline'\"><title>t</title><body><p id=x>x</p>"
        ),
    }
    with sync_playwright() as p:
        browser = p.chromium.launch()
        for name, html in pages.items():
            page = browser.new_page()
            page.set_content(html)
            page.evaluate(script)
            page.evaluate(script)  # a second page load must replace, not duplicate
            assert page.evaluate("document.querySelectorAll('#rustcord-theme').length") == 1, name
            assert page.evaluate("getComputedStyle(document.body).backgroundColor") == "rgb(1, 2, 3)", name
            assert page.evaluate("getComputedStyle(document.documentElement).getPropertyValue('--rustcord-accent').trim()") == "#5865f2", name
            assert page.evaluate("window.pwned") is None, name
            assert page.evaluate("document.querySelectorAll('script').length") == 0, name
            page.close()
        browser.close()
    print("theme script: applies, is idempotent and cannot inject markup in real Chromium")


if __name__ == "__main__":
    check_png()
    check_theme_script()
    sys.exit(0)
