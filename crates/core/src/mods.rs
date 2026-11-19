//! Vencord / Equicord support: opt-in loader for the projects' own standalone builds.
//!
//! The official `*.user.js` builds are self-contained and only need `unsafeWindow` and
//! `GM_xmlhttpRequest`, so Rustcord runs them unmodified inside a small wrapper at document
//! start. Plugins, settings and themes then behave as in the browser extension (settings live
//! in the page's localStorage / IndexedDB, which the CEF profile persists).
//!
//! Limits of the userscript builds: plugins that need the desktop `native` layer are
//! unavailable, and cross-origin requests go through a `fetch` shim.
//! Pure logic, no CEF.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Refuse absurd bundle sizes (Equicord is about 3 MB).
pub const MAX_BUNDLE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModClient {
    Vencord,
    Equicord,
}

impl ModClient {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "vencord" => Some(Self::Vencord),
            "equicord" => Some(Self::Equicord),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Vencord => "vencord",
            Self::Equicord => "equicord",
        }
    }
    pub fn download_url(self) -> &'static str {
        match self {
            Self::Vencord => {
                "https://github.com/Vendicated/Vencord/releases/download/devbuild/Vencord.user.js"
            }
            Self::Equicord => {
                "https://github.com/Equicord/Equicord/releases/download/latest/Equicord.user.js"
            }
        }
    }
    /// Text every genuine build contains; guards against saving an HTML error page.
    fn marker(self) -> &'static str {
        match self {
            Self::Vencord => "Vencord",
            Self::Equicord => "Equicord",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub client: ModClient,
    pub url: String,
    pub sha256: String,
    pub size: usize,
    pub installed_unix: u64,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn dir(data_dir: &Path, c: ModClient) -> PathBuf {
    data_dir.join("mods").join(c.name())
}
pub fn script_path(data_dir: &Path, c: ModClient) -> PathBuf {
    dir(data_dir, c).join("bundle.user.js")
}
fn manifest_path(data_dir: &Path, c: ModClient) -> PathBuf {
    dir(data_dir, c).join("manifest.json")
}

pub fn manifest(data_dir: &Path, c: ModClient) -> Option<Manifest> {
    serde_json::from_slice(&fs::read(manifest_path(data_dir, c)).ok()?).ok()
}

/// Check downloaded bytes look like the expected bundle.
pub fn validate(c: ModClient, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > MAX_BUNDLE_BYTES {
        return Err(format!("unexpected size: {} bytes", bytes.len()));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "not UTF-8 text".to_string())?;
    let head = text.trim_start();
    if head.starts_with('<') || !text.contains(c.marker()) || !text.contains("unsafeWindow") {
        return Err("does not look like a userscript build".into());
    }
    Ok(())
}

/// Store a verified download plus a manifest recording its SHA-256.
pub fn install(data_dir: &Path, c: ModClient, bytes: &[u8], url: &str) -> Result<Manifest, String> {
    validate(c, bytes)?;
    let d = dir(data_dir, c);
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    write_atomic(&script_path(data_dir, c), bytes).map_err(|e| e.to_string())?;
    let m = Manifest {
        client: c,
        url: url.to_string(),
        sha256: sha256_hex(bytes),
        size: bytes.len(),
        installed_unix: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    };
    let json = serde_json::to_vec_pretty(&m).map_err(|e| e.to_string())?;
    write_atomic(&manifest_path(data_dir, c), &json).map_err(|e| e.to_string())?;
    Ok(m)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".tmp{}", std::process::id()));
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Load the script to inject. `custom` (the user's own build, e.g. with userplugins) is
/// used as is; an installed bundle must still match the hash recorded at install time.
pub fn load_script(data_dir: &Path, c: ModClient, custom: Option<&Path>) -> Result<String, String> {
    if let Some(p) = custom {
        let bytes = fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        if bytes.len() > MAX_BUNDLE_BYTES {
            return Err("custom bundle too large".into());
        }
        return String::from_utf8(bytes).map_err(|_| "custom bundle is not UTF-8".into());
    }
    let m = manifest(data_dir, c).ok_or_else(|| {
        format!(
            "{} not installed: run `rustcord mods install {}`",
            c.name(),
            c.name()
        )
    })?;
    let bytes = fs::read(script_path(data_dir, c)).map_err(|e| e.to_string())?;
    if sha256_hex(&bytes) != m.sha256 {
        return Err(format!(
            "{} bundle does not match its recorded hash; reinstall it",
            c.name()
        ));
    }
    String::from_utf8(bytes).map_err(|_| "bundle is not UTF-8".into())
}

/// Wrap a userscript build for evaluation at document start in the page's main world.
/// `GM_xmlhttpRequest` is a `fetch` shim (subject to CORS), which covers the requests
/// the clients make to hosts that allow cross-origin access.
pub fn wrap_script(src: &str) -> String {
    format!(
        "(function(){{if(window.__rustcordMod)return;window.__rustcordMod=true;\
var unsafeWindow=window;\
function GM_xmlhttpRequest(o){{var c=new AbortController();\
fetch(o.url,{{method:o.method||'GET',headers:o.headers,body:o.data,signal:c.signal}})\
.then(function(r){{return (o.responseType==='blob'?r.blob():o.responseType==='arraybuffer'?r.arrayBuffer():r.text()).then(function(b){{\
var h=[];r.headers.forEach(function(v,k){{h.push(k+': '+v)}});\
if(o.onload)o.onload({{status:r.status,statusText:r.statusText,response:b,responseText:typeof b==='string'?b:'',finalUrl:r.url,responseHeaders:h.join('\\r\\n')}});}});}})\
.catch(function(e){{if(o.onerror)o.onerror(e)}});\
return {{abort:function(){{c.abort()}}}};}}\n{src}\n}})();"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rustcord-mods-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    const GOOD: &[u8] = b"// ==UserScript==\nunsafeWindow.Vencord={};";

    #[test]
    fn parse_names() {
        assert_eq!(ModClient::parse("Vencord"), Some(ModClient::Vencord));
        assert_eq!(ModClient::parse("equicord"), Some(ModClient::Equicord));
        assert_eq!(ModClient::parse("x"), None);
    }

    #[test]
    fn validate_rejects_html_and_wrong_client() {
        assert!(validate(ModClient::Vencord, GOOD).is_ok());
        assert!(validate(ModClient::Equicord, GOOD).is_err());
        assert!(validate(ModClient::Vencord, b"<html>Vencord unsafeWindow</html>").is_err());
        assert!(validate(ModClient::Vencord, b"").is_err());
    }

    #[test]
    fn install_then_load_and_tamper_detection() {
        let d = tmp("install");
        let m = install(&d, ModClient::Vencord, GOOD, "u").unwrap();
        assert_eq!(m.sha256, sha256_hex(GOOD));
        assert_eq!(manifest(&d, ModClient::Vencord).unwrap(), m);
        assert!(load_script(&d, ModClient::Vencord, None)
            .unwrap()
            .contains("Vencord"));
        fs::write(script_path(&d, ModClient::Vencord), b"evil").unwrap();
        assert!(load_script(&d, ModClient::Vencord, None)
            .unwrap_err()
            .contains("hash"));
    }

    #[test]
    fn missing_install_and_custom_path() {
        let d = tmp("custom");
        assert!(load_script(&d, ModClient::Equicord, None)
            .unwrap_err()
            .contains("not installed"));
        let p = d.join("mine.js");
        fs::write(&p, "var x=1;").unwrap();
        assert_eq!(
            load_script(&d, ModClient::Equicord, Some(&p)).unwrap(),
            "var x=1;"
        );
    }

    #[test]
    fn sha256_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn wrapper_defines_shims_and_embeds_source() {
        let w = wrap_script("window.hit=unsafeWindow===window;");
        assert!(w.contains("function GM_xmlhttpRequest") && w.contains("window.hit="));
    }
}
