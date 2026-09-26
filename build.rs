// SPDX-License-Identifier: GPL-3.0-or-later

fn main() {
    // libass renders text subtitles (SRT, ASS, WebVTT) for PGS conversion.
    pkg_config::Config::new().atleast_version("0.15").probe("libass").expect("libass development files are required");
}
