#!/usr/bin/env python3
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Run the real Vencord/Equicord userscript builds, wrapped exactly as Rustcord wraps them,
in headless Chromium against a stub discord.com page. Usage: verify_mods.py <bundle.user.js>..."""
import subprocess, sys, pathlib
from playwright.sync_api import sync_playwright

root = pathlib.Path(__file__).resolve().parent.parent
ok = True
with sync_playwright() as pw:
    b = pw.chromium.launch()
    for path in sys.argv[1:]:
        src = pathlib.Path(path).read_bytes()
        wrapped = subprocess.run(
            ["cargo", "run", "-q", "-p", "rustcord-core", "--example", "dump", "--", "mod-script"],
            input=src, capture_output=True, check=True, cwd=root).stdout.decode()
        page = b.new_page()
        errs = []
        page.on("pageerror", lambda e: errs.append(str(e)))
        page.route("https://discord.com/**", lambda r: r.fulfill(
            content_type="text/html", body="<!doctype html><html><head></head><body></body></html>"))
        page.add_init_script(wrapped)
        page.goto("https://discord.com/app")
        page.wait_for_timeout(2000)
        res = page.evaluate("""() => ({
            api: typeof window.Vencord?.Api, plugins: Object.keys(window.Vencord?.Plugins?.plugins ?? {}).length,
            settings: Object.keys(localStorage).filter(k => /Settings$/.test(k))})""")
        good = res["api"] == "object" and res["plugins"] > 20 and res["settings"] and not errs
        print(("PASS" if good else "FAIL"), path, res, errs[:3])
        ok &= bool(good)
    b.close()
sys.exit(0 if ok else 1)
