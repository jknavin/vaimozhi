//! Model files: listing, explicit download with progress/resume, integrity checks, deletion.
//!
//! Nothing here runs unless the user clicks Download. Integrity works in two layers:
//!  1. At download time the server's reported SHA-256 (Hugging Face exposes it as the
//!     `x-linked-etag` header of the redirect) is compared with the downloaded file.
//!  2. The computed hash is stored in `<model>.sha256`; "Verify" recomputes and compares it
//!     later, which catches truncation and disk corruption.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use vaimozhi_core::models::{self, ModelSpec};

#[derive(Debug, Clone, Serialize)]
pub struct ModelStatus {
    pub id: String,
    pub name: String,
    pub size_mb: u32,
    pub ram_mb: u32,
    pub quantized: bool,
    pub note: String,
    pub installed: bool,
    pub installed_bytes: u64,
    pub has_partial: bool,
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    pub ok: bool,
    pub sha256: String,
    pub message: String,
}

pub fn path_for(dir: &Path, spec: &ModelSpec) -> PathBuf {
    dir.join(spec.file)
}

fn sidecar(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".sha256");
    PathBuf::from(s)
}

fn partial(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub fn list(dir: &Path, recommended_id: &str) -> Vec<ModelStatus> {
    models::CATALOG
        .iter()
        .map(|m| {
            let p = path_for(dir, m);
            let meta = std::fs::metadata(&p).ok();
            ModelStatus {
                id: m.id.into(),
                name: m.name.into(),
                size_mb: m.size_mb,
                ram_mb: m.ram_mb,
                quantized: m.quantized,
                note: m.note.into(),
                installed: meta.is_some(),
                installed_bytes: meta.map(|m| m.len()).unwrap_or(0),
                has_partial: partial(&p).exists(),
                recommended: m.id == recommended_id,
            }
        })
        .collect()
}

pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub fn verify(dir: &Path, spec: &ModelSpec) -> VerifyResult {
    let p = path_for(dir, spec);
    if !p.exists() {
        return VerifyResult { ok: false, sha256: String::new(), message: "Model is not installed.".into() };
    }
    let actual = match hash_file(&p) {
        Ok(h) => h,
        Err(e) => return VerifyResult { ok: false, sha256: String::new(), message: format!("Could not read file: {e}") },
    };
    match std::fs::read_to_string(sidecar(&p)) {
        Ok(expected) if expected.trim() == actual => VerifyResult {
            ok: true,
            sha256: actual,
            message: "File matches the hash recorded at download time.".into(),
        },
        Ok(_) => VerifyResult {
            ok: false,
            sha256: actual,
            message: "File does NOT match the recorded hash. Delete it and download again.".into(),
        },
        Err(_) => {
            // No sidecar (for example the user copied the file in by hand): record the hash now.
            let _ = std::fs::write(sidecar(&p), &actual);
            VerifyResult {
                ok: true,
                sha256: actual,
                message: "No earlier hash existed; this one was recorded. Compare it with the publisher's hash if you need certainty.".into(),
            }
        }
    }
}

pub fn delete(dir: &Path, spec: &ModelSpec) -> std::io::Result<()> {
    let p = path_for(dir, spec);
    for f in [p.clone(), sidecar(&p), partial(&p)] {
        if f.exists() {
            std::fs::remove_file(f)?;
        }
    }
    Ok(())
}

pub fn free_space_bytes(dir: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    disks
        .iter()
        .filter(|d| canon.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

fn reference_hash(url: &str) -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .ok()?;
    let resp = client.head(url).send().ok()?;
    for name in ["x-linked-etag", "etag"] {
        if let Some(v) = resp.headers().get(name).and_then(|v| v.to_str().ok()) {
            let v = v.trim().trim_start_matches("W/").trim_matches('"').to_lowercase();
            if v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()) {
                return Some(v);
            }
        }
    }
    None
}

/// Blocking download with resume. `progress(done_bytes, total_bytes)` is throttled by the caller.
pub fn download(
    dir: &Path,
    spec: &ModelSpec,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<VerifyResult, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create models folder: {e}"))?;
    let final_path = path_for(dir, spec);
    let part = partial(&final_path);
    let already = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);

    let needed = (spec.size_mb as u64 * 1_048_576).saturating_sub(already);
    if let Some(free) = free_space_bytes(dir) {
        if free < needed + 200 * 1_048_576 {
            return Err(format!(
                "Not enough free disk space: about {} MB needed, {} MB available.",
                needed / 1_048_576 + 200,
                free / 1_048_576
            ));
        }
    }

    let url = models::download_url(spec);
    let expected = reference_hash(&url);

    let client = reqwest::blocking::Client::builder()
        .timeout(None)
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let mut req = client.get(&url);
    if already > 0 {
        req = req.header("Range", format!("bytes={already}-"));
    }
    let mut resp = req.send().map_err(|e| format!("Download failed: {e}. Check your internet connection."))?;
    let status = resp.status();
    let resumed = status.as_u16() == 206;
    if !status.is_success() {
        return Err(format!("Download failed: server answered {status}."));
    }
    let remaining = resp.content_length().unwrap_or(0);
    let total = if resumed { already + remaining } else { remaining };
    let mut done = if resumed { already } else { 0 };

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed)
        .truncate(!resumed)
        .open(&part)
        .map_err(|e| format!("cannot write model file: {e}"))?;

    let mut buf = vec![0u8; 256 * 1024];
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into()); // keep the .part file so the download can resume
        }
        let n = resp.read(&mut buf).map_err(|e| format!("Download interrupted: {e}. Click Download again to resume."))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| format!("cannot write model file: {e}"))?;
        done += n as u64;
        progress(done, total);
    }
    file.flush().map_err(|e| e.to_string())?;
    drop(file);

    if total > 0 && done != total {
        return Err("Download ended early. Click Download again to resume.".into());
    }

    let actual = hash_file(&part).map_err(|e| e.to_string())?;
    if let Some(exp) = &expected {
        if *exp != actual {
            let _ = std::fs::remove_file(&part);
            return Err("Integrity check failed: the downloaded file does not match the publisher's SHA-256. It was deleted; try again.".into());
        }
    }
    std::fs::rename(&part, &final_path).map_err(|e| e.to_string())?;
    std::fs::write(sidecar(&final_path), &actual).map_err(|e| e.to_string())?;

    Ok(VerifyResult {
        ok: true,
        sha256: actual,
        message: if expected.is_some() {
            "Downloaded and verified against the publisher's SHA-256.".into()
        } else {
            "Downloaded. The server did not publish a hash, so only a local hash was recorded.".into()
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_and_verify_roundtrip() {
        let dir = std::env::temp_dir().join(format!("vaimozhi-models-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let spec = models::find("tiny").unwrap();
        let p = path_for(&dir, spec);
        std::fs::write(&p, b"abc").unwrap();
        // SHA-256("abc")
        assert_eq!(
            hash_file(&p).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(verify(&dir, spec).ok); // records the hash
        assert!(verify(&dir, spec).ok); // matches
        std::fs::write(&p, b"abd").unwrap();
        assert!(!verify(&dir, spec).ok); // corruption detected
        delete(&dir, spec).unwrap();
        assert!(!p.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
