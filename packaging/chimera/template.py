# SPDX-License-Identifier: LGPL-3.0-or-later
# DRAFT cports template for Chimera Linux (Qt WebEngine frontend). NOT BUILT OR TESTED:
# it was written without access to a Chimera system or the cports tree.
#
# Before it can work:
#   1. Tag a release (v0.1.0) in a repo cports can fetch, and set `url` below.
#   2. Replace `sha256` (`sha256sum v0.1.0.tar.gz`).
#   3. Put it at user/rustcord/template.py in your cports checkout and run
#      ./cbuild pkg user/rustcord  (check names against `./cbuild` and existing
#      templates: build-style options, dependency names and cargo vendoring change).
pkgname = "rustcord"
pkgver = "0.1.0"
pkgrel = 0
build_style = "cargo"
# Only the Qt frontend: the CEF one needs glibc-only binaries.
make_build_args = ["-p", "rustcord-qt"]
make_install_args = ["--path", "crates/qt"]
make_check_args = ["-p", "rustcord-core"]
hostmakedepends = ["cargo-auto-vendor", "pkgconf"]
makedepends = ["qt6-qtbase-devel", "qt6-qtwebengine-devel", "rust-std"]
depends = ["qt6-qtwebengine"]
pkgdesc = "Native Discord client hosting the web app in Qt WebEngine"
license = "LGPL-3.0-or-later"
url = "https://github.com/CHANGE-ME/rustcord"
source = f"{url}/archive/refs/tags/v{pkgver}.tar.gz"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"


def post_install(self):
    self.install_file("packaging/rustcord-qt.desktop", "usr/share/applications")
    self.install_file(
        "packaging/icons/rustcord-256.png",
        "usr/share/icons/hicolor/256x256/apps",
        name="rustcord.png",
    )
    self.install_license("COPYING.LESSER")
    self.install_license("COPYING")
