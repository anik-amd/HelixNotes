// WebDAV sync provider.
//
// Model: the vault stays local; this mirrors it to/from a user-configured WebDAV
// remote (Nextcloud/ownCloud/NAS). A local manifest (.helixnotes/sync_state.json)
// records what was in sync last time, so we can do a three-way diff (local vs remote
// vs manifest) and resolve each file as upload/download/delete, with keep-both
// conflict copies so nothing is ever lost.
//
// Synced set: every `*.md` in the vault tree, `.helixnotes/attachments/`,
// `.helixnotes/notebook_icons.json`, and `.helixnotes/tag_styles.json`. Search indexes,
// trash, history, other metadata, and the manifest itself remain local-only.

use crate::state::AppState;
use crate::vault::operations::helixnotes_dir;
use crate::vault::watcher::changed_entry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use tauri::{Emitter, Manager};
use walkdir::WalkDir;

/// Folder listings requested at once unless the settings say otherwise. A handful hides most of
/// the round-trip time and stays polite to a small server.
pub const DEFAULT_LISTING_PARALLELISM: usize = 6;
/// The most a setting can ask for.
pub const MAX_LISTING_PARALLELISM: usize = 16;

/// What a stored setting means: `0` (never chosen) is the default, and nothing exceeds the
/// maximum. The one place these rules live.
pub fn effective_listing_parallelism(setting: usize) -> usize {
    match setting {
        0 => DEFAULT_LISTING_PARALLELISM,
        n => n.min(MAX_LISTING_PARALLELISM),
    }
}

/// A listing error that asking again cannot fix: a wrong login, a folder that is not there, or
/// a client error the server will answer the same way. Anything else, such as a dropped
/// connection, a timeout, 429 or a 5xx, may pass.
fn is_final_listing_error(error: &str) -> bool {
    if error.starts_with("Authentication failed") || error.starts_with("WebDAV path not found") {
        return true;
    }
    error
        .strip_prefix("WebDAV PROPFIND failed: HTTP ")
        .and_then(|code| code.trim().parse::<u16>().ok())
        .is_some_and(|code| (400..500).contains(&code) && code != 408 && code != 429)
}

// ───────────────────────── config ─────────────────────────

pub struct WebdavConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    /// Folders listed on the server at once: `0` for the default, `1` for one at a time.
    pub listing_parallelism: usize,
}

#[derive(Default, Clone, Serialize)]
pub struct SyncSummary {
    pub uploaded: u32,
    pub downloaded: u32,
    pub deleted_local: u32,
    pub deleted_remote: u32,
    pub conflicts: u32,
}

// ───────────────────────── manifest ─────────────────────────

#[derive(Default, Serialize, Deserialize)]
struct SyncManifest {
    #[serde(default)]
    identity: String,
    files: HashMap<String, ManifestEntry>, // relpath (forward-slash) -> entry
}

#[derive(Clone, Serialize, Deserialize)]
struct ManifestEntry {
    local_hash: String,  // sha256 of local content at last sync
    remote_etag: String, // strong remote etag at last sync, or empty
    #[serde(default)]
    remote_hash: String, // sha256 of remote content when no strong etag is available
}

fn manifest_path(vault: &str) -> PathBuf {
    helixnotes_dir(vault).join("sync_state.json")
}

/// A local filesystem failure, named as such with the operation and the file. The bare OS error
/// ("Operation not permitted (os error 1)") names no file and reads like a server problem.
fn local_error(operation: &str, path: impl AsRef<Path>, error: impl std::fmt::Display) -> String {
    format!(
        "Local file error: cannot {operation} \"{}\": {error}",
        path.as_ref().display()
    )
}

/// A request that never got an answer from the server (connection, TLS, timeout).
fn webdav_error(operation: &str, relpath: &str, error: impl std::fmt::Display) -> String {
    format!("WebDAV error: cannot {operation} \"{relpath}\": {error}")
}

/// The only relative paths sync will touch: plain file and folder names separated by `/`.
/// A remote listing is untrusted input, and so is a stored manifest: a `..` segment, an absolute
/// or drive-prefixed path, or a backslash must never be joined to the vault folder.
fn validate_relpath(rel: &str) -> Result<(), String> {
    validate_relpath_for(rel, cfg!(windows))
}

fn validate_relpath_for(rel: &str, windows: bool) -> Result<(), String> {
    let reject = |reason: &str| {
        Err(format!(
            "Sync stopped: \"{}\" is not a valid file path ({reason}). No file was written for it.",
            rel.escape_debug()
        ))
    };
    if rel.is_empty() {
        return reject("empty path");
    }
    for segment in rel.split('/') {
        if segment.is_empty() {
            return reject("empty path segment");
        }
        if segment == "." || segment == ".." {
            return reject("relative path segment");
        }
        if segment.contains('\\') {
            return reject("backslash");
        }
        if segment.chars().any(char::is_control) {
            return reject("control character");
        }
        // Windows reads `D:x` as a drive path and `x:y` as a file stream, and drops trailing dots
        // and spaces, which can turn a segment into `..`.
        if windows && (segment.contains(':') || segment.ends_with('.') || segment.ends_with(' ')) {
            return reject("not a valid Windows file name");
        }
    }
    Ok(())
}

/// The local file for a synced path. Every join of a sync key to the vault goes through here.
fn local_path(vault: &Path, relpath: &str) -> Result<PathBuf, String> {
    validate_relpath(relpath)?;
    Ok(vault.join(relpath))
}

/// Forgets what the last sync saw. After a backup restore the vault no longer matches it: kept,
/// the next sync would read the rollback as local edits, delete everything newer on the server and
/// overwrite newer notes there with the restored ones. Without it the next sync starts over as a
/// first sync, which merges both sides and deletes nothing.
pub(crate) fn forget_baseline(vault: &str) -> Result<(), String> {
    let path = manifest_path(vault);
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(local_error("reset sync state", &path, error)),
    }
}

fn load_manifest(vault: &str) -> Result<SyncManifest, String> {
    match fs::read(manifest_path(vault)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("Invalid sync manifest; sync stopped to protect your notes: {e}")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(SyncManifest::default()),
        Err(error) => Err(format!("Cannot read sync manifest: {error}")),
    }
}

fn save_manifest(vault: &str, m: &SyncManifest) -> Result<(), String> {
    let bytes = serde_json::to_vec(m).map_err(|e| e.to_string())?;
    let target = manifest_path(vault);
    let folder = target.parent().unwrap();
    fs::create_dir_all(folder).map_err(|e| local_error("create folder", folder, e))?;
    crate::vault::operations::replace_file_atomically(&target, &bytes)
        .map_err(|e| local_error("save sync state", &target, e))
}

fn baseline_for_endpoint(manifest: SyncManifest, identity: &str) -> SyncManifest {
    if manifest.identity == identity {
        manifest
    } else {
        // Legacy manifests without an identity are also untrusted: they may
        // belong to another server. Reconcile as two independent snapshots.
        SyncManifest {
            identity: identity.to_string(),
            files: HashMap::new(),
        }
    }
}

// ───────────────────────── helpers ─────────────────────────

const HASH_BUFFER_SIZE: usize = 64 * 1024;

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn sha256_reader(
    reader: &mut impl Read,
    buffer: &mut [u8; HASH_BUFFER_SIZE],
) -> io::Result<String> {
    let mut hasher = Sha256::new();
    loop {
        match reader.read(buffer) {
            Ok(0) => return Ok(format!("{:x}", hasher.finalize())),
            Ok(count) => hasher.update(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn strong_etag(s: &str) -> String {
    let s = s.trim();
    if s.starts_with("W/") || s.len() < 2 || !s.starts_with('"') || !s.ends_with('"') {
        String::new()
    } else {
        s.to_string()
    }
}

/// Percent-encode each path segment, keeping the `/` separators.
fn encode_path(relpath: &str) -> String {
    relpath
        .split('/')
        .map(|seg| urlencoding::encode(seg).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Build the conflict-copy relpath: `dir/Note (conflict <stamp>).md`.
fn conflict_relpath(relpath: &str, stamp: &str) -> String {
    let (dir, name) = match relpath.rfind('/') {
        Some(i) => (&relpath[..=i], &relpath[i + 1..]),
        None => ("", relpath),
    };
    let (stem, ext) = match name.rfind('.') {
        Some(i) => (&name[..i], &name[i..]),
        None => (name, ""),
    };
    format!("{}{} (conflict {}){}", dir, stem, stamp, ext)
}

// The note list, search and quick open all go by a note's title, so a conflict copy that only
// differs in its file name looks identical to the original. Give the copy the same suffix in its
// title. A note without a title of its own already takes it from the file name.
fn conflict_copy(relpath: &str, bytes: Vec<u8>, stamp: &str) -> Vec<u8> {
    use crate::vault::{frontmatter, operations};
    if !relpath.ends_with(".md") {
        return bytes;
    }
    let Ok(raw) = std::str::from_utf8(&bytes) else {
        return bytes;
    };
    if frontmatter::extract_title(raw).is_none() {
        return bytes;
    }
    let filename = relpath.rsplit('/').next().unwrap_or(relpath);
    let (mut meta, body) = frontmatter::parse_note(raw, filename);
    let title = format!("{} (conflict {stamp})", meta.title);
    let body = operations::retitle_leading_heading(&body, &meta.title, &title);
    meta.title = title;
    frontmatter::merge_frontmatter(raw, &meta, &body).into_bytes()
}

fn write_local(
    vault: &Path,
    relpath: &str,
    bytes: &[u8],
    changes: &mut HashMap<PathBuf, &'static str>,
) -> Result<(), String> {
    let target = local_path(vault, relpath)?;
    if fs::read(&target).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| local_error("create folder for", relpath, e))?;
    }
    if matches!(
        relpath,
        ".helixnotes/notebook_icons.json" | ".helixnotes/tag_styles.json"
    ) {
        let content = std::str::from_utf8(bytes).map_err(|error| error.to_string())?;
        crate::vault::operations::attachment_metadata_references(
            target.file_name().and_then(|name| name.to_str()).unwrap(),
            content,
        )?;
    }
    crate::vault::operations::replace_file_atomically(&target, bytes)
        .map_err(|e| local_error("write", relpath, e))?;
    changes.insert(target, "modify");
    Ok(())
}

fn require_unchanged_local(path: &Path, expected_hash: Option<&str>) -> Result<(), String> {
    let actual = match fs::File::open(path) {
        Ok(mut file) => Some(
            sha256_reader(&mut file, &mut [0; HASH_BUFFER_SIZE])
                .map_err(|error| local_error("read", path, error))?,
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(local_error("read", path, error)),
    };
    if actual.as_deref() != expected_hash {
        return Err(format!(
            "Sync stopped because {} changed locally during sync",
            path.display()
        ));
    }
    Ok(())
}

fn write_local_if_unchanged(
    write_guard: &tokio::sync::Mutex<()>,
    vault: &Path,
    relpath: &str,
    bytes: &[u8],
    expected_hash: Option<&str>,
    changes: &mut HashMap<PathBuf, &'static str>,
) -> Result<(), String> {
    let _guard = write_guard.blocking_lock();
    require_unchanged_local(&local_path(vault, relpath)?, expected_hash)?;
    write_local(vault, relpath, bytes, changes)
}

struct LocalFile {
    hash: String,
    path: PathBuf,
}

/// The synced set: `*.md` anywhere outside `.helixnotes/`, everything under
/// `.helixnotes/attachments/`, plus notebook icon and tag style mappings. Applied to BOTH local
/// and remote so pointing at a folder with unrelated files never imports them.
fn is_synced_relpath(rel: &str) -> bool {
    // On a case-insensitive disk `.HelixNotes/history/x.md` lands in the app's own folder, so any
    // other spelling of it is never synced, whatever lies below.
    let first = rel.split('/').next().unwrap_or_default();
    if first != ".helixnotes"
        && (first.to_lowercase() == ".helixnotes" || first.to_uppercase() == ".HELIXNOTES")
    {
        return false;
    }
    if rel.starts_with(".helixnotes/") {
        rel.starts_with(".helixnotes/attachments/")
            || rel == ".helixnotes/notebook_icons.json"
            || rel == ".helixnotes/tag_styles.json"
    } else {
        rel.ends_with(".md")
    }
}

fn scan_local(vault: &str) -> Result<HashMap<String, LocalFile>, String> {
    let vault_path = Path::new(vault);
    let mut map = HashMap::new();
    let mut buffer = [0; HASH_BUFFER_SIZE];
    for entry in WalkDir::new(vault_path) {
        let entry = entry.map_err(|e| format!("Cannot scan local vault: {e}"))?;
        let p = entry.path();
        // WalkDir deliberately does not follow links. A note replaced by a
        // link (or a linked directory of notes) must not look like a deletion
        // and authorize a remote DELETE. Do not traverse outside the vault.
        if entry.file_type().is_symlink() {
            return Err(format!(
                "Cannot sync vault containing symbolic link {}",
                p.display()
            ));
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            return Err(format!(
                "Cannot sync unsupported filesystem entry {}",
                p.display()
            ));
        }
        let rel = p
            .strip_prefix(vault_path)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        if !is_synced_relpath(&rel) {
            continue;
        }
        let hash = fs::File::open(p)
            .and_then(|mut file| sha256_reader(&mut file, &mut buffer))
            .map_err(|e| format!("Cannot read local file {}: {e}", p.display()))?;
        map.insert(
            rel,
            LocalFile {
                hash,
                path: p.to_path_buf(),
            },
        );
    }
    Ok(map)
}

// ───────────────────────── WebDAV client ─────────────────────────

struct RemoteEntry {
    relpath: String,
    etag: String,
    is_dir: bool,
}

struct WebdavClient {
    base: String,      // normalized, no trailing slash
    base_path: String, // path component of base, no trailing slash (percent-encoded)
    client: reqwest::blocking::Client,
    user: String,
    pass: String,
    listing_parallelism: usize,
}

impl WebdavClient {
    fn new(cfg: WebdavConfig) -> Result<Self, String> {
        let mut url =
            reqwest::Url::parse(cfg.url.trim()).map_err(|e| format!("Invalid WebDAV URL: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err("WebDAV URL must start with http:// or https://".to_string());
        }
        if url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("WebDAV URL must not contain credentials, query, or fragment".to_string());
        }
        let path = url.path().trim_end_matches('/').to_string();
        url.set_path(if path.is_empty() { "/" } else { &path });
        let base = url.to_string().trim_end_matches('/').to_string();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base,
            base_path: url.path().trim_end_matches('/').to_string(),
            client,
            user: cfg.username,
            pass: cfg.password,
            listing_parallelism: effective_listing_parallelism(cfg.listing_parallelism),
        })
    }

    fn identity(&self) -> String {
        sha256_hex(format!("{}\n{}", self.base, self.user).as_bytes())
    }

    fn url_for(&self, relpath: &str) -> String {
        if relpath.is_empty() {
            self.base.clone()
        } else {
            format!("{}/{}", self.base, encode_path(relpath))
        }
    }

    fn propfind(&self, relpath: &str, depth: &str) -> Result<Vec<RemoteEntry>, String> {
        let method = reqwest::Method::from_bytes(b"PROPFIND").unwrap();
        let body = r#"<?xml version="1.0" encoding="utf-8"?><d:propfind xmlns:d="DAV:"><d:prop><d:getetag/><d:resourcetype/></d:prop></d:propfind>"#;
        let resp = self
            .client
            .request(method, self.url_for(relpath))
            .basic_auth(&self.user, Some(&self.pass))
            .header("Depth", depth)
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body)
            .send()
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.as_u16() == 401 {
            return Err("Authentication failed (check username / app password)".to_string());
        }
        if status.as_u16() == 404 {
            return Err("WebDAV path not found (check the URL)".to_string());
        }
        if !status.is_success() {
            return Err(format!("WebDAV PROPFIND failed: HTTP {}", status.as_u16()));
        }
        let text = resp.text().map_err(|e| e.to_string())?;
        parse_multistatus(&text, &self.base_path)
    }

    /// Recursively list all files (relpath -> etag) by walking collections at Depth 1
    /// (Depth: infinity is disabled on some servers, so we walk instead).
    fn list(&self) -> Result<HashMap<String, String>, String> {
        let mut files = HashMap::new();
        let mut visited: HashSet<String> = HashSet::from([String::new()]);
        // One level of folders at a time: every folder of a level is listed at once.
        let mut level: Vec<String> = vec![String::new()];
        while !level.is_empty() {
            let listings = self.list_folders(&level)?;
            let mut next = Vec::new();
            for (dir, entries) in level.iter().zip(listings) {
                for e in entries {
                    if e.relpath == *dir {
                        continue; // the collection itself
                    }
                    if e.is_dir {
                        if visited.insert(e.relpath.clone()) {
                            next.push(e.relpath);
                        }
                    } else {
                        files.insert(e.relpath, e.etag);
                    }
                }
            }
            level = next;
        }
        Ok(files)
    }

    /// The contents of each folder, in the order given, with up to `listing_parallelism`
    /// requests in flight. Once a request fails the others stop taking new folders. A folder
    /// that failed, or was not reached, is then listed on its own, one after another, unless
    /// its error is final: a server that dislikes concurrent requests costs time, not the sync,
    /// while a wrong login or a missing folder is reported at once.
    fn list_folders(&self, dirs: &[String]) -> Result<Vec<Vec<RemoteEntry>>, String> {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let workers = dirs.len().min(self.listing_parallelism);
        if workers <= 1 {
            return dirs.iter().map(|dir| self.propfind(dir, "1")).collect();
        }
        let next = AtomicUsize::new(0);
        let failed = AtomicBool::new(false);
        let mut listed: Vec<Option<Result<Vec<RemoteEntry>, String>>> =
            dirs.iter().map(|_| None).collect();
        std::thread::scope(|scope| {
            let threads: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        let mut done = Vec::new();
                        loop {
                            if failed.load(Ordering::Relaxed) {
                                break done;
                            }
                            let at = next.fetch_add(1, Ordering::Relaxed);
                            let Some(dir) = dirs.get(at) else {
                                break done;
                            };
                            let result = self.propfind(dir, "1");
                            if result.is_err() {
                                failed.store(true, Ordering::Relaxed);
                            }
                            done.push((at, result));
                        }
                    })
                })
                .collect();
            for thread in threads {
                for (at, result) in thread.join().expect("a listing thread does not panic") {
                    listed[at] = Some(result);
                }
            }
        });
        listed
            .into_iter()
            .zip(dirs)
            .map(|(result, dir)| match result {
                Some(Ok(entries)) => Ok(entries),
                Some(Err(error)) if is_final_listing_error(&error) => Err(error),
                Some(Err(_)) | None => self.propfind(dir, "1"),
            })
            .collect()
    }

    fn get(&self, relpath: &str, expected_etag: &str) -> Result<Vec<u8>, String> {
        let mut request = self
            .client
            .get(self.url_for(relpath))
            .basic_auth(&self.user, Some(&self.pass));
        if !expected_etag.is_empty() {
            request = request.header("If-Match", expected_etag);
        }
        let resp = request
            .send()
            .map_err(|e| webdav_error("download", relpath, e))?;
        if resp.status().as_u16() == 412 {
            return Err(format!(
                "Remote file {relpath} changed during sync; retry sync"
            ));
        }
        if !resp.status().is_success() {
            return Err(format!(
                "GET {relpath} failed: HTTP {}",
                resp.status().as_u16()
            ));
        }
        let response_etag = resp
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok())
            .map(strong_etag)
            .unwrap_or_default();
        if !expected_etag.is_empty() && !response_etag.is_empty() && response_etag != expected_etag
        {
            return Err(format!(
                "Remote file {relpath} changed during sync; retry sync"
            ));
        }
        let bytes: Vec<u8> = resp.bytes().map_err(|e| e.to_string())?.into();
        if !expected_etag.is_empty() && self.etag_for(relpath)? != expected_etag {
            return Err(format!(
                "Remote file {relpath} changed during sync; retry sync"
            ));
        }
        Ok(bytes)
    }

    fn put(&self, relpath: &str, bytes: &[u8], prior_etag: Option<&str>) -> Result<(), String> {
        let mut request = self
            .client
            .put(self.url_for(relpath))
            .basic_auth(&self.user, Some(&self.pass))
            .body(bytes.to_vec());
        request = match prior_etag {
            Some(etag) if !etag.is_empty() => request.header("If-Match", etag),
            Some(_) => {
                return Err(format!(
                    "Cannot safely overwrite {relpath}: server provides no strong ETag"
                ))
            }
            None => request.header("If-None-Match", "*"),
        };
        let resp = request
            .send()
            .map_err(|e| webdav_error("upload", relpath, e))?;
        if resp.status().as_u16() == 412 {
            return Err(format!(
                "Remote file {relpath} changed during sync; retry sync"
            ));
        }
        if !resp.status().is_success() {
            return Err(format!(
                "PUT {relpath} failed: HTTP {}",
                resp.status().as_u16()
            ));
        }
        Ok(())
    }

    fn delete(&self, relpath: &str, etag: &str) -> Result<(), String> {
        if etag.is_empty() {
            return Err(format!(
                "Cannot safely delete {relpath}: server provides no strong ETag"
            ));
        }
        let resp = self
            .client
            .delete(self.url_for(relpath))
            .basic_auth(&self.user, Some(&self.pass))
            .header("If-Match", etag)
            .send()
            .map_err(|e| webdav_error("delete", relpath, e))?;
        if resp.status().as_u16() == 412 || resp.status().as_u16() == 404 {
            return Err(format!(
                "Remote file {relpath} changed during sync; retry sync"
            ));
        }
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "DELETE {relpath} failed: HTTP {}",
                resp.status().as_u16()
            ))
        }
    }

    fn mkcol(&self, relpath: &str) -> Result<(), String> {
        let method = reqwest::Method::from_bytes(b"MKCOL").unwrap();
        let resp = self
            .client
            .request(method, self.url_for(relpath))
            .basic_auth(&self.user, Some(&self.pass))
            .send()
            .map_err(|e| webdav_error("create folder", relpath, e))?;
        let s = resp.status().as_u16();
        if resp.status().is_success() || s == 405 || s == 301 || s == 403 {
            Ok(())
        } else {
            Err(format!("MKCOL {relpath} failed: HTTP {s}"))
        }
    }

    fn ensure_dirs(&self, relpath: &str, created: &mut HashSet<String>) -> Result<(), String> {
        let parts: Vec<&str> = relpath.split('/').collect();
        if parts.len() <= 1 {
            return Ok(());
        }
        let mut cur = String::new();
        for part in &parts[..parts.len() - 1] {
            if !cur.is_empty() {
                cur.push('/');
            }
            cur.push_str(part);
            if created.insert(cur.clone()) {
                self.mkcol(&cur)?;
            }
        }
        Ok(())
    }

    fn etag_for(&self, relpath: &str) -> Result<String, String> {
        self.propfind(relpath, "0")?
            .into_iter()
            .find(|e| e.relpath == relpath && !e.is_dir)
            .map(|e| e.etag)
            .ok_or_else(|| format!("Remote file {relpath} disappeared during sync; retry sync"))
    }

    fn upload(
        &self,
        relpath: &str,
        bytes: &[u8],
        prior_etag: Option<&str>,
        created: &mut HashSet<String>,
    ) -> Result<String, String> {
        self.ensure_dirs(relpath, created)?;
        self.put(relpath, bytes, prior_etag)?;
        let etag = self.etag_for(relpath)?;
        if self.get(relpath, &etag)?.as_slice() != bytes {
            return Err(format!(
                "Remote file {relpath} changed after upload; retry sync"
            ));
        }
        Ok(etag)
    }
}

/// Map a WebDAV `href` to a vault-relative path (forward slash).
/// `Ok(None)` is the base collection itself. `Err` means the href could not be mapped
/// (e.g. it does not sit under the configured base path, as some providers like Infomaniak
/// kDrive return). Callers MUST treat `Err` as a hard failure: silently dropping an
/// unmappable href makes that file look deleted on the remote, and the reconciler then
/// wipes the local copy. (issue #102)
fn href_to_relpath(href: &str, base_path: &str) -> Result<Option<String>, String> {
    // Pull out just the path component (some servers return absolute-URL hrefs).
    let raw = if href.starts_with("http://") || href.starts_with("https://") {
        reqwest::Url::parse(href)
            .map_err(|e| format!("invalid remote href URL '{href}': {e}"))?
            .path()
            .to_string()
    } else {
        href.to_string()
    };
    // Compare DECODED paths, not the raw percent-encoded strings. Providers differ in how they
    // encode hrefs: Infomaniak kDrive lowercases the hex (`%5b`/`%5d`) and encodes spaces, while
    // reqwest emits uppercase hex (`%5B`/`%5D`) for base_path. A raw strip_prefix then spuriously
    // fails, every remote file looks deleted, and the reconciler wipes the local copies. Decoding
    // both sides first makes the prefix match encoding-agnostic. (issue #102)
    let path = urlencoding::decode(raw.trim_end_matches('/'))
        .map_err(|e| format!("bad percent-encoding in remote path '{raw}': {e}"))?
        .into_owned();
    let base = urlencoding::decode(base_path.trim_end_matches('/'))
        .map_err(|e| format!("bad percent-encoding in WebDAV base path '{base_path}': {e}"))?
        .into_owned();
    let rest = match path.strip_prefix(&base) {
        // `/dav-other/x` is not inside `/dav`: the folder must end at a path boundary.
        Some(r) if r.is_empty() || r.starts_with('/') || base.is_empty() => {
            r.trim_start_matches('/')
        }
        _ => {
            return Err(format!(
                "remote path '{path}' is not under the configured WebDAV folder '{base}'. \
                 Aborting before deleting anything locally - please report this URL."
            ))
        }
    };
    if rest.is_empty() {
        return Ok(None); // the base collection itself
    }
    validate_relpath(rest)?;
    Ok(Some(rest.to_string()))
}

fn parse_multistatus(xml: &str, base_path: &str) -> Result<Vec<RemoteEntry>, String> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    #[derive(PartialEq)]
    enum Cap {
        None,
        Href,
        Etag,
    }

    let mut reader = Reader::from_str(xml);
    let mut entries = Vec::new();
    let mut href: Option<String> = None;
    let mut etag: Option<String> = None;
    let mut is_dir = false;
    let mut cap = Cap::None;
    let mut buf = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"response" => {
                    href = None;
                    etag = None;
                    is_dir = false;
                }
                b"href" => {
                    cap = Cap::Href;
                    buf.clear();
                }
                b"getetag" => {
                    cap = Cap::Etag;
                    buf.clear();
                }
                b"collection" => is_dir = true,
                _ => {}
            },
            Ok(Event::Empty(e)) => {
                if e.local_name().as_ref() == b"collection" {
                    is_dir = true;
                }
            }
            Ok(Event::Text(e)) => {
                if cap != Cap::None {
                    let text = e
                        .decode()
                        .map_err(|e| format!("XML text decode error: {e}"))?;
                    buf.push_str(&text);
                }
            }
            Ok(Event::CData(e)) => {
                if cap != Cap::None {
                    let text = e
                        .decode()
                        .map_err(|e| format!("XML CDATA decode error: {e}"))?;
                    buf.push_str(&text);
                }
            }
            Ok(Event::GeneralRef(e)) => {
                if cap != Cap::None {
                    if let Some(ch) = e
                        .resolve_char_ref()
                        .map_err(|e| format!("XML reference error: {e}"))?
                    {
                        buf.push(ch);
                    } else {
                        let name = e
                            .decode()
                            .map_err(|e| format!("XML reference decode error: {e}"))?;
                        let text = quick_xml::escape::resolve_xml_entity(&name)
                            .ok_or_else(|| format!("Unknown XML entity: &{name};"))?;
                        buf.push_str(text);
                    }
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"href" => {
                    href = Some(buf.clone());
                    cap = Cap::None;
                }
                b"getetag" => {
                    etag = Some(buf.clone());
                    cap = Cap::None;
                }
                b"response" => {
                    if let Some(h) = href.take() {
                        if let Some(rel) = href_to_relpath(&h, base_path)? {
                            entries.push(RemoteEntry {
                                relpath: rel,
                                etag: etag.take().map(|s| strong_etag(&s)).unwrap_or_default(),
                                is_dir,
                            });
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }
    Ok(entries)
}

// ───────────────────────── sync engine ─────────────────────────

fn read_scanned_local(file: &LocalFile) -> Result<Vec<u8>, String> {
    let bytes =
        fs::read(&file.path).map_err(|e| format!("Cannot read {}: {e}", file.path.display()))?;
    if sha256_hex(&bytes) != file.hash {
        return Err(format!(
            "Sync stopped because {} changed locally during sync",
            file.path.display()
        ));
    }
    Ok(bytes)
}

fn manifest_entry(hash: String, etag: &str) -> ManifestEntry {
    ManifestEntry {
        remote_hash: hash.clone(),
        local_hash: hash,
        remote_etag: etag.to_string(),
    }
}

/// `fetched_hash` is the remote content read because the server has no ETag or reports a
/// different one. Only different bytes are a remote change: some servers hand out a new ETag
/// for a file nobody touched, and a local edit would then become a conflict copy.
fn remote_changed(old: Option<&ManifestEntry>, fetched_hash: Option<&str>) -> bool {
    match (old, fetched_hash) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(old), Some(hash)) => old.remote_hash.is_empty() || old.remote_hash != hash,
    }
}

fn apply_changes(
    client: &WebdavClient,
    vault: &str,
    local: &HashMap<String, LocalFile>,
    remote: &HashMap<String, String>,
    manifest: &SyncManifest,
    changes: &mut HashMap<PathBuf, &'static str>,
    write_guard: &tokio::sync::Mutex<()>,
) -> Result<(SyncManifest, SyncSummary), String> {
    let vault_path = Path::new(vault);
    let mut new_m = SyncManifest {
        identity: client.identity(),
        files: HashMap::new(),
    };
    let mut sum = SyncSummary::default();
    let mut created: HashSet<String> = HashSet::new();
    let stamp = chrono::Local::now().format("%Y-%m-%d %H-%M-%S").to_string();
    let mut keys: HashSet<&String> = HashSet::new();
    keys.extend(local.keys());
    keys.extend(remote.keys());
    keys.extend(manifest.files.keys());

    for key in keys {
        let l = local.get(key);
        let r = remote.get(key);
        let m = manifest.files.get(key);
        match (l, r) {
            (Some(lf), Some(etag)) => {
                // On ETag-less servers, a listing cannot tell us if content changed.
                // Read every time and compare to the last accepted remote content hash.
                let remote_bytes =
                    if etag.is_empty() || m.is_none_or(|old| old.remote_etag != *etag) {
                        Some(client.get(key, etag)?)
                    } else {
                        None
                    };
                let remote_hash = remote_bytes.as_ref().map(|bytes| sha256_hex(bytes));
                let remote_changed = remote_changed(m, remote_hash.as_deref());
                let local_changed = m.is_none_or(|old| old.local_hash != lf.hash);
                if !local_changed && !remote_changed {
                    new_m.files.insert(
                        key.clone(),
                        ManifestEntry {
                            local_hash: lf.hash.clone(),
                            remote_etag: etag.clone(),
                            remote_hash: m.unwrap().remote_hash.clone(),
                        },
                    );
                } else if local_changed && !remote_changed {
                    let bytes = read_scanned_local(lf)?;
                    let new_etag = client.upload(key, &bytes, Some(etag), &mut created)?;
                    new_m
                        .files
                        .insert(key.clone(), manifest_entry(lf.hash.clone(), &new_etag));
                    sum.uploaded += 1;
                } else {
                    let bytes = match remote_bytes {
                        Some(bytes) => bytes,
                        None => client.get(key, etag)?,
                    };
                    let hash = remote_hash.unwrap_or_else(|| sha256_hex(&bytes));
                    if hash == lf.hash {
                        new_m.files.insert(key.clone(), manifest_entry(hash, etag));
                    } else if !local_changed {
                        write_local_if_unchanged(
                            write_guard,
                            vault_path,
                            key,
                            &bytes,
                            Some(&lf.hash),
                            changes,
                        )?;
                        new_m.files.insert(key.clone(), manifest_entry(hash, etag));
                        sum.downloaded += 1;
                    } else {
                        // Keep both, but only replace the canonical note after the
                        // previous local bytes have been safely copied aside.
                        let ckey = conflict_relpath(key, &stamp);
                        let lbytes = {
                            let _guard = write_guard.blocking_lock();
                            require_unchanged_local(&local_path(vault_path, &ckey)?, None)?;
                            let lbytes = conflict_copy(&ckey, read_scanned_local(lf)?, &stamp);
                            write_local(vault_path, &ckey, &lbytes, changes)?;
                            write_local(vault_path, key, &bytes, changes)?;
                            lbytes
                        };
                        new_m.files.insert(key.clone(), manifest_entry(hash, etag));
                        let cetag = client.upload(&ckey, &lbytes, None, &mut created)?;
                        new_m
                            .files
                            .insert(ckey, manifest_entry(sha256_hex(&lbytes), &cetag));
                        sum.conflicts += 1;
                    }
                }
            }
            (Some(lf), None) => match m {
                None => {
                    let bytes = read_scanned_local(lf)?;
                    let etag = client.upload(key, &bytes, None, &mut created)?;
                    new_m
                        .files
                        .insert(key.clone(), manifest_entry(lf.hash.clone(), &etag));
                    sum.uploaded += 1;
                }
                Some(me) if me.local_hash != lf.hash => {
                    let bytes = read_scanned_local(lf)?;
                    let etag = client.upload(key, &bytes, None, &mut created)?;
                    new_m
                        .files
                        .insert(key.clone(), manifest_entry(lf.hash.clone(), &etag));
                    sum.uploaded += 1;
                }
                Some(_) => {
                    let _guard = write_guard.blocking_lock();
                    require_unchanged_local(&lf.path, Some(&lf.hash))?;
                    fs::remove_file(&lf.path).map_err(|e| local_error("delete", &lf.path, e))?;
                    changes.insert(lf.path.clone(), "remove");
                    sum.deleted_local += 1;
                }
            },
            (None, Some(etag)) => {
                let bytes = if etag.is_empty() || m.is_none_or(|old| old.remote_etag != *etag) {
                    Some(client.get(key, etag)?)
                } else {
                    None
                };
                let remote_hash = bytes.as_ref().map(|bytes| sha256_hex(bytes));
                let remote_changed = remote_changed(m, remote_hash.as_deref());
                if remote_changed {
                    let bytes = match bytes {
                        Some(bytes) => bytes,
                        None => client.get(key, etag)?,
                    };
                    write_local_if_unchanged(write_guard, vault_path, key, &bytes, None, changes)?;
                    new_m.files.insert(
                        key.clone(),
                        manifest_entry(remote_hash.unwrap_or_else(|| sha256_hex(&bytes)), etag),
                    );
                    sum.downloaded += 1;
                } else {
                    client.delete(key, etag)?;
                    sum.deleted_remote += 1;
                }
            }
            (None, None) => {}
        }
    }
    Ok((new_m, sum))
}

/// Validate the connection (auth + URL) with a cheap PROPFIND.
pub fn test_connection(cfg: WebdavConfig) -> Result<String, String> {
    let client = WebdavClient::new(cfg)?;
    client.propfind("", "0")?;
    Ok("Connection successful".to_string())
}

/// Run a full sync. Mutes the file watcher while applying local writes, then
/// queues only changed notes for search and publishes their final local state.
pub fn run_sync(
    app: tauri::AppHandle,
    vault: String,
    cfg: WebdavConfig,
) -> Result<SyncSummary, String> {
    use std::sync::atomic::Ordering;

    let state = app.state::<AppState>();
    let _maintenance = state.vault_maintenance.blocking_lock();
    // Keep the original vault's index even if the user switches vaults while
    // WebDAV is working. No config lock is held during network or file I/O.
    let search = state.vault_snapshot(&vault)?;
    let client = WebdavClient::new(cfg)?;

    let _ = app.emit(
        "sync-progress",
        serde_json::json!({ "vault_path": vault, "status": "listing" }),
    );
    let remote: HashMap<String, String> = client
        .list()?
        .into_iter()
        .filter(|(k, _)| is_synced_relpath(k))
        .collect();
    let (local, manifest) = {
        let _guard = state.note_save.blocking_lock();
        (
            scan_local(&vault)?,
            baseline_for_endpoint(load_manifest(&vault)?, &client.identity()),
        )
    };

    // Safety guard (issue #102): never treat an empty remote listing as "everything was
    // deleted remotely". If a prior sync exists (non-empty manifest) but the remote came back
    // empty, it is almost certainly a listing failure (e.g. a provider whose paths we can't
    // map), not a real mass-deletion - so abort instead of wiping local notes.
    if remote.is_empty() && !manifest.files.is_empty() {
        return Err(
            "Sync aborted to protect your notes: the WebDAV server returned an empty file list \
             even though files were synced before, so no local files were deleted. This usually \
             means the server reports paths in a format the app can't read yet - please report \
             your WebDAV provider and folder URL."
                .to_string(),
        );
    }

    let _ = app.emit(
        "sync-progress",
        serde_json::json!({ "vault_path": vault, "status": "syncing" }),
    );
    let mut changes = HashMap::new();
    // Mute the watcher across local writes; its deferred batch also preserves
    // external edits that arrive while the network operation is in progress.
    {
        let _guard = state.note_save.blocking_lock();
        state.importing.store(true, Ordering::Release);
    }
    let applied = apply_changes(
        &client,
        &vault,
        &local,
        &remote,
        &manifest,
        &mut changes,
        &state.note_save,
    );
    state.importing.store(false, Ordering::Release);

    // Publish even a partial apply: a later network failure must not leave
    // successfully downloaded/deleted notes absent from search or the UI.
    let active = state
        .config
        .lock()
        .ok()
        .is_some_and(|config| config.active_vault.as_deref() == Some(vault.as_str()));
    let metadata_root = helixnotes_dir(&vault);
    for (path, event_type) in changes {
        let is_note = path.extension().and_then(|extension| extension.to_str()) == Some("md")
            && !path.starts_with(&metadata_root);
        if is_note {
            if let Some(search) = &search {
                if let Err(error) = search.queue_index_note(&path.to_string_lossy()) {
                    log::error!("Could not queue synced note for search: {error}");
                }
            }
        }
        if !active {
            continue;
        }
        let event = if is_note {
            changed_entry(path, event_type, Path::new(&vault))
        } else {
            crate::types::FileEvent {
                vault_path: vault.clone(),
                event_type: event_type.to_string(),
                path: path.to_string_lossy().to_string(),
                note: None,
            }
        };
        let _ = app.emit("file-changed", event);
    }

    let (new_manifest, summary) = applied?;
    save_manifest(&vault, &new_manifest)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WebDAV server for listing tests: `folders` folders below the root, each holding
    /// `note.md`, every answer coming after `delay` like a server far away. `status` is given the
    /// folder's path and how many requests were in progress as this one arrived (itself
    /// included), and may answer with an error status instead of the listing. Returns the URL
    /// and a count of the requests received.
    fn listing_server(
        folders: usize,
        delay: std::time::Duration,
        status: impl Fn(&str, usize) -> Option<u16> + Send + Sync + 'static,
    ) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/vault", listener.local_addr().unwrap());
        let status = Arc::new(status);
        let requests = Arc::new(AtomicUsize::new(0));
        let in_progress = Arc::new(AtomicUsize::new(0));
        let counted = requests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let mut stream = stream.unwrap();
                let (status, requests, in_progress) =
                    (status.clone(), requests.clone(), in_progress.clone());
                std::thread::spawn(move || {
                    // The whole request, body included: closing with part of it unread can
                    // reset the connection before the client has read the answer.
                    let mut request = Vec::new();
                    let mut chunk = [0; 4096];
                    let header_end = loop {
                        let len = stream.read(&mut chunk).unwrap();
                        assert!(len > 0);
                        request.extend_from_slice(&chunk[..len]);
                        if let Some(at) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                            break at + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
                    let content_length: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse().ok())
                        })
                        .unwrap_or(0);
                    while request.len() < header_end + content_length {
                        let len = stream.read(&mut chunk).unwrap();
                        assert!(len > 0);
                        request.extend_from_slice(&chunk[..len]);
                    }
                    let path = headers
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/")
                        .trim_end_matches('/')
                        .to_string();
                    requests.fetch_add(1, Ordering::SeqCst);
                    let concurrent = in_progress.fetch_add(1, Ordering::SeqCst) + 1;
                    std::thread::sleep(delay);
                    in_progress.fetch_sub(1, Ordering::SeqCst);
                    let reply = match status(&path, concurrent) {
                        Some(code) => format!(
                            "HTTP/1.1 {code} Refused\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        ),
                        None => {
                            let entry = |href: &str, collection: bool| {
                                format!(
                                    "<d:response><d:href>{href}</d:href><d:propstat><d:prop>\
                                     <d:getetag>\"e\"</d:getetag><d:resourcetype>{}</d:resourcetype>\
                                     </d:prop></d:propstat></d:response>",
                                    if collection { "<d:collection/>" } else { "" }
                                )
                            };
                            let mut body = String::new();
                            if path == "/vault" {
                                body += &entry("/vault/", true);
                                for n in 1..=folders {
                                    body += &entry(&format!("/vault/f{n}/"), true);
                                }
                            } else {
                                body += &entry(&format!("{path}/"), true);
                                body += &entry(&format!("{path}/note.md"), false);
                            }
                            let body =
                                format!("<d:multistatus xmlns:d=\"DAV:\">{body}</d:multistatus>");
                            format!(
                                "HTTP/1.1 207 Multi-Status\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                body.len()
                            )
                        }
                    };
                    stream.write_all(reply.as_bytes()).unwrap();
                });
            }
        });
        (url, counted)
    }

    fn listing_client(url: String, listing_parallelism: usize) -> WebdavClient {
        WebdavClient::new(WebdavConfig {
            url,
            username: String::new(),
            password: String::new(),
            listing_parallelism,
        })
        .unwrap()
    }

    fn note_paths(folders: usize) -> Vec<String> {
        let mut expected: Vec<String> = (1..=folders).map(|n| format!("f{n}/note.md")).collect();
        expected.sort();
        expected
    }

    fn listed_names(client: &WebdavClient) -> Vec<String> {
        let mut found: Vec<_> = client.list().unwrap().into_keys().collect();
        found.sort();
        found
    }

    #[test]
    fn folders_are_listed_at_once_by_default_and_the_result_is_complete() {
        const FOLDERS: usize = 12;
        const DELAY: std::time::Duration = std::time::Duration::from_millis(150);
        let (url, _) = listing_server(FOLDERS, DELAY, |_, _| None);
        let client = listing_client(url, 0);

        let started = std::time::Instant::now();
        let found = listed_names(&client);
        let took = started.elapsed();

        assert_eq!(found, note_paths(FOLDERS));
        // One after another this takes (1 + FOLDERS) * DELAY = 1.95 s; the root and then two
        // rounds of six take about 0.45 s.
        assert!(
            took < std::time::Duration::from_millis(1200),
            "listing took {took:?}"
        );
    }

    #[test]
    fn a_setting_of_one_lists_one_folder_at_a_time_with_the_same_result() {
        const FOLDERS: usize = 4;
        const DELAY: std::time::Duration = std::time::Duration::from_millis(100);
        let (url, _) = listing_server(FOLDERS, DELAY, |_, _| None);
        let client = listing_client(url, 1);

        let started = std::time::Instant::now();
        let found = listed_names(&client);
        let took = started.elapsed();

        assert_eq!(found, note_paths(FOLDERS));
        // The root and each folder, one after another.
        assert!(
            took >= DELAY * (FOLDERS as u32 + 1),
            "listing took {took:?}"
        );
    }

    #[test]
    fn a_server_that_refuses_concurrent_requests_still_lists_everything() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        const FOLDERS: usize = 4;
        let refused = Arc::new(AtomicUsize::new(0));
        let counted = refused.clone();
        // 503 for a request that arrives while another is being served.
        let (url, _) = listing_server(
            FOLDERS,
            std::time::Duration::from_millis(60),
            move |_, concurrent| {
                (concurrent > 1).then(|| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    503
                })
            },
        );
        let client = listing_client(url, 4);

        assert_eq!(listed_names(&client), note_paths(FOLDERS));
        // The fallback really ran: with four requests at once, at least one was refused.
        assert!(refused.load(Ordering::SeqCst) >= 1);
    }

    #[test]
    fn a_final_error_stops_the_listing_at_once_without_asking_again() {
        const FOLDERS: usize = 30;
        // The first folder answers 403 every time.
        let (url, requests) =
            listing_server(FOLDERS, std::time::Duration::from_millis(100), |path, _| {
                path.ends_with("/f1").then_some(403)
            });
        let client = listing_client(url, 6);

        let error = client.list().unwrap_err();

        assert!(error.contains("HTTP 403"), "{error}");
        // The root, the six first folders, and no more: the rest were never asked for, and the
        // refused one was not repeated.
        let received = requests.load(std::sync::atomic::Ordering::SeqCst);
        assert!(received < 14, "{received} requests for {FOLDERS} folders");
    }

    #[test]
    fn only_errors_that_another_attempt_could_fix_are_retried() {
        for final_error in [
            "Authentication failed (check username / app password)",
            "WebDAV path not found (check the URL)",
            "WebDAV PROPFIND failed: HTTP 403",
            "WebDAV PROPFIND failed: HTTP 404",
            "WebDAV PROPFIND failed: HTTP 410",
        ] {
            assert!(is_final_listing_error(final_error), "{final_error}");
        }
        for passing in [
            "WebDAV PROPFIND failed: HTTP 429",
            "WebDAV PROPFIND failed: HTTP 408",
            "WebDAV PROPFIND failed: HTTP 500",
            "WebDAV PROPFIND failed: HTTP 503",
            "error sending request for url (http://x.test/dav): connection reset",
            "operation timed out",
        ] {
            assert!(!is_final_listing_error(passing), "{passing}");
        }
    }

    #[test]
    fn the_listing_setting_means_default_when_zero_and_never_exceeds_the_maximum() {
        assert_eq!(
            effective_listing_parallelism(0),
            DEFAULT_LISTING_PARALLELISM
        );
        assert_eq!(effective_listing_parallelism(1), 1);
        assert_eq!(effective_listing_parallelism(3), 3);
        assert_eq!(
            effective_listing_parallelism(MAX_LISTING_PARALLELISM),
            MAX_LISTING_PARALLELISM
        );
        assert_eq!(effective_listing_parallelism(1000), MAX_LISTING_PARALLELISM);
        let parallelism =
            |setting| listing_client("http://x.test".into(), setting).listing_parallelism;
        assert_eq!(parallelism(0), DEFAULT_LISTING_PARALLELISM);
        assert_eq!(parallelism(1000), MAX_LISTING_PARALLELISM);
    }

    #[test]
    fn a_conflict_copy_says_so_in_its_title() {
        let raw =
            "---\nid: abc\ntitle: Plan\ntags:\n- work\ncustom: kept\n---\n# Plan\n\nLocal text.\n";
        let copy = conflict_copy(
            "Work/Plan (conflict 2026-10-11 10-00-00).md",
            raw.as_bytes().to_vec(),
            "2026-10-11 10-00-00",
        );
        let copy = String::from_utf8(copy).unwrap();
        assert!(
            copy.contains("title: Plan (conflict 2026-10-11 10-00-00)\n"),
            "{copy}"
        );
        assert!(
            copy.contains("# Plan (conflict 2026-10-11 10-00-00)\n\nLocal text."),
            "{copy}"
        );
        assert!(
            copy.contains("custom: kept") && copy.contains("- work") && copy.contains("id: abc"),
            "{copy}"
        );

        // No title of its own: the file name already names the copy. Other files are untouched.
        let untitled = b"Just text.\n".to_vec();
        assert_eq!(
            conflict_copy("Note (conflict x).md", untitled.clone(), "x"),
            untitled
        );
        let image = vec![0xff, 0xd8, 0x00];
        assert_eq!(
            conflict_copy("a (conflict x).jpg", image.clone(), "x"),
            image
        );
        let binary_md = vec![0xff, 0xfe];
        assert_eq!(
            conflict_copy("b (conflict x).md", binary_md.clone(), "x"),
            binary_md
        );
    }

    #[test]
    fn synced_metadata_waits_for_local_patches_and_never_overwrites_a_newer_style() {
        use crate::types::TagStyleChange;
        use crate::vault::operations;
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path().to_str().unwrap();
        operations::ensure_vault_structure(root).unwrap();
        operations::patch_tag_style(root, "Work", TagStyleChange::Color(Some("#123456".into())))
            .unwrap();
        let rel = ".helixnotes/tag_styles.json";
        let path = vault.path().join(rel);
        let hash = sha256_hex(&fs::read(&path).unwrap());
        let guard = tokio::sync::Mutex::new(());
        let held = guard.blocking_lock();
        std::thread::scope(|scope| {
            let pending = scope.spawn(|| {
                let mut changes = HashMap::new();
                let result = write_local_if_unchanged(
                    &guard,
                    vault.path(),
                    rel,
                    br##"{"Work":{"color":"#ffffff"}}"##,
                    Some(&hash),
                    &mut changes,
                );
                assert!(result.is_err());
                assert!(changes.is_empty());
            });
            operations::patch_tag_style(
                root,
                "Work",
                TagStyleChange::Icon(Some("builtin:star".into())),
            )
            .unwrap();
            drop(held);
            pending.join().unwrap();
        });
        let persisted = operations::load_tag_styles(root).unwrap();
        assert_eq!(persisted["Work"].icon.as_deref(), Some("builtin:star"));
        assert_eq!(persisted["Work"].color.as_deref(), Some("#123456"));

        let current = fs::read(&path).unwrap();
        let mut original = fs::File::open(&path).unwrap();
        let mut changes = HashMap::new();
        write_local_if_unchanged(
            &guard,
            vault.path(),
            rel,
            br##"{"Work":{"color":"#abcdef","icon":"builtin:star"}}"##,
            Some(&sha256_hex(&current)),
            &mut changes,
        )
        .unwrap();
        let mut old_bytes = Vec::new();
        original.read_to_end(&mut old_bytes).unwrap();
        assert_eq!(old_bytes, current);
        assert_eq!(
            operations::load_tag_styles(root).unwrap()["Work"]
                .color
                .as_deref(),
            Some("#abcdef")
        );

        let before = fs::read(&path).unwrap();
        assert!(write_local_if_unchanged(
            &guard,
            vault.path(),
            rel,
            b"{broken",
            Some(&sha256_hex(&before)),
            &mut HashMap::new()
        )
        .is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
    }

    #[test]
    fn streaming_hash_matches_bytes_across_buffer_boundaries() {
        let mut buffer = [0; HASH_BUFFER_SIZE];
        for len in [0, 256, HASH_BUFFER_SIZE, HASH_BUFFER_SIZE * 2 + 137] {
            let bytes: Vec<u8> = (0..len).map(|i| (i % 256) as u8).collect();
            assert_eq!(
                sha256_reader(&mut bytes.as_slice(), &mut buffer).unwrap(),
                sha256_hex(&bytes),
                "hash mismatch for {len} bytes"
            );
        }
    }

    #[test]
    fn streaming_hash_retries_interruptions_and_accepts_short_reads() {
        struct InterruptedShortReader<'a> {
            remaining: &'a [u8],
            interrupt_next: bool,
        }

        impl Read for InterruptedShortReader<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.interrupt_next = !self.interrupt_next;
                if self.interrupt_next {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let len = buffer.len().min(7);
                self.remaining.read(&mut buffer[..len])
            }
        }

        let bytes: Vec<u8> = (0..513).map(|i| (i % 256) as u8).collect();
        let mut reader = InterruptedShortReader {
            remaining: &bytes,
            interrupt_next: false,
        };
        let mut buffer = [0; HASH_BUFFER_SIZE];
        assert_eq!(
            sha256_reader(&mut reader, &mut buffer).unwrap(),
            sha256_hex(&bytes)
        );
    }

    #[test]
    fn streaming_hash_rejects_partial_content_after_read_error() {
        struct FailingReader;

        impl Read for FailingReader {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::ErrorKind::PermissionDenied.into())
            }
        }

        let mut reader = b"partial content".as_slice().chain(FailingReader);
        let mut buffer = [0; HASH_BUFFER_SIZE];
        assert_eq!(
            sha256_reader(&mut reader, &mut buffer).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn local_scan_hashes_large_attachments_and_independent_files() {
        let vault = tempfile::tempdir().unwrap();
        fs::create_dir_all(vault.path().join(".helixnotes/attachments")).unwrap();
        let attachment: Vec<u8> = (0..HASH_BUFFER_SIZE * 2 + 31)
            .map(|i| (i % 256) as u8)
            .collect();
        let files = [
            (".helixnotes/attachments/large.bin", attachment.as_slice()),
            (
                ".helixnotes/attachments/small.bin",
                b"\0\xff\x80".as_slice(),
            ),
            ("empty.md", b"".as_slice()),
            ("note.md", b"note content".as_slice()),
        ];
        for (relpath, bytes) in files {
            fs::write(vault.path().join(relpath), bytes).unwrap();
        }
        fs::write(vault.path().join("ignored.bin"), b"not synced").unwrap();

        let local = scan_local(vault.path().to_str().unwrap()).unwrap();
        let actual: HashMap<_, _> = local
            .into_iter()
            .map(|(relpath, file)| (relpath, (file.hash, file.path)))
            .collect();
        let expected: HashMap<_, _> = files
            .into_iter()
            .map(|(relpath, bytes)| {
                (
                    relpath.to_string(),
                    (sha256_hex(bytes), vault.path().join(relpath)),
                )
            })
            .collect();
        assert_eq!(actual, expected);
    }

    #[cfg(unix)]
    #[test]
    fn previously_synced_note_replaced_by_link_is_not_scanned_as_deleted() {
        use std::os::unix::fs::symlink;
        let vault = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = vault.path().to_str().unwrap();
        let note = vault.path().join("note.md");
        fs::write(&note, b"accepted").unwrap();
        let baseline = scan_local(root).unwrap();
        assert_eq!(baseline["note.md"].hash, sha256_hex(b"accepted"));

        let external = outside.path().join("outside.md");
        fs::write(&external, b"external").unwrap();
        fs::remove_file(&note).unwrap();
        symlink(&external, &note).unwrap();
        assert!(scan_local(root).err().unwrap().contains("symbolic link"));
        assert_eq!(fs::read(&external).unwrap(), b"external");

        fs::remove_file(&note).unwrap();
        symlink(outside.path().join("missing.md"), &note).unwrap();
        assert!(scan_local(root).err().unwrap().contains("symbolic link"));

        fs::remove_file(&note).unwrap();
        fs::create_dir_all(outside.path().join("Notebook")).unwrap();
        fs::write(outside.path().join("Notebook/note.md"), b"external").unwrap();
        symlink(
            outside.path().join("Notebook"),
            vault.path().join("Notebook"),
        )
        .unwrap();
        assert!(scan_local(root).err().unwrap().contains("symbolic link"));
        assert_eq!(
            fs::read(outside.path().join("Notebook/note.md")).unwrap(),
            b"external"
        );
    }

    #[test]
    fn syncs_notebook_icon_mapping_and_assets_only() {
        for path in [
            "Notes/plan.md",
            ".helixnotes/attachments/notebook-icon.png",
            ".helixnotes/notebook_icons.json",
            ".helixnotes/tag_styles.json",
        ] {
            assert!(is_synced_relpath(path), "expected {path} to be synced");
        }

        for path in [
            "Notes/image.png",
            ".helixnotes/sync_state.json",
            ".helixnotes/notebook_icons.json.bak",
            ".helixnotes/tag_styles.json.bak",
            ".helixnotes/attachments-old/icon.png",
        ] {
            assert!(!is_synced_relpath(path), "expected {path} to stay local");
        }
    }

    #[test]
    fn unchanged_sync_has_no_local_mutations() {
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("note.md");
        fs::write(&path, "unchanged content").unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let vault_path = vault.path().to_str().unwrap();
        let local = scan_local(vault_path).unwrap();
        let remote = HashMap::from([("note.md".to_string(), "\"etag\"".to_string())]);
        let manifest = SyncManifest {
            identity: String::new(),
            files: HashMap::from([(
                "note.md".to_string(),
                ManifestEntry {
                    local_hash: local["note.md"].hash.clone(),
                    remote_etag: "\"etag\"".to_string(),
                    remote_hash: String::new(),
                },
            )]),
        };
        let client = WebdavClient::new(WebdavConfig {
            url: "http://127.0.0.1:0/".to_string(),
            username: String::new(),
            password: String::new(),
            listing_parallelism: 0,
        })
        .unwrap();
        let mut changes = HashMap::new();
        let (next, summary) = apply_changes(
            &client,
            vault_path,
            &local,
            &remote,
            &manifest,
            &mut changes,
            &tokio::sync::Mutex::new(()),
        )
        .unwrap();
        assert!(changes.is_empty());
        assert_eq!(summary.uploaded, 0);
        assert_eq!(summary.downloaded, 0);
        assert_eq!(summary.deleted_local, 0);
        assert_eq!(summary.deleted_remote, 0);
        assert_eq!(summary.conflicts, 0);
        assert_eq!(next.files["note.md"].local_hash, local["note.md"].hash);
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);
        assert_eq!(fs::read_to_string(path).unwrap(), "unchanged content");
    }

    #[test]
    fn remote_deletion_records_only_the_removed_local_note() {
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("note.md");
        fs::write(&path, "unchanged content").unwrap();
        let vault_path = vault.path().to_str().unwrap();
        let local = scan_local(vault_path).unwrap();
        let manifest = SyncManifest {
            identity: String::new(),
            files: HashMap::from([(
                "note.md".to_string(),
                ManifestEntry {
                    local_hash: local["note.md"].hash.clone(),
                    remote_etag: "\"etag\"".to_string(),
                    remote_hash: String::new(),
                },
            )]),
        };
        let client = WebdavClient::new(WebdavConfig {
            url: "http://127.0.0.1:0/".to_string(),
            username: String::new(),
            password: String::new(),
            listing_parallelism: 0,
        })
        .unwrap();
        let mut changes = HashMap::new();
        let (next, summary) = apply_changes(
            &client,
            vault_path,
            &local,
            &HashMap::new(),
            &manifest,
            &mut changes,
            &tokio::sync::Mutex::new(()),
        )
        .unwrap();
        assert!(!path.exists());
        assert!(!next.files.contains_key("note.md"));
        assert_eq!(summary.deleted_local, 1);
        assert_eq!(changes, HashMap::from([(path, "remove")]));
    }

    #[test]
    fn local_download_journal_ignores_identical_bytes_and_preserves_partial_changes() {
        let vault = tempfile::tempdir().unwrap();
        let path = vault.path().join("Notes/note.md");
        let mut changes = HashMap::new();
        write_local(
            vault.path(),
            "Notes/note.md",
            b"first content",
            &mut changes,
        )
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first content");
        changes.clear();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        write_local(
            vault.path(),
            "Notes/note.md",
            b"first content",
            &mut changes,
        )
        .unwrap();
        assert!(changes.is_empty());
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), modified);

        write_local(
            vault.path(),
            "Notes/note.md",
            b"remote content",
            &mut changes,
        )
        .unwrap();
        fs::write(vault.path().join("blocked"), b"not a directory").unwrap();
        let error = write_local(
            vault.path(),
            "blocked/note.md",
            b"cannot write",
            &mut changes,
        )
        .unwrap_err();
        // The failure names itself as local, the operation and the file (issue #249).
        assert!(
            error.starts_with("Local file error: cannot create folder for \"blocked/note.md\": "),
            "{error}"
        );
        assert_eq!(changes, HashMap::from([(path.clone(), "modify")]));
        let event = changed_entry(path, "modify", vault.path());
        assert_eq!(event.note.unwrap().preview, "remote content");
    }
    #[test]
    fn forgetting_the_baseline_makes_the_next_sync_a_first_sync() {
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path().to_str().unwrap();
        let mut recorded = SyncManifest::default();
        recorded.files.insert(
            "gone.md".to_string(),
            manifest_entry("hash".to_string(), "\"etag\""),
        );
        save_manifest(root, &recorded).unwrap();
        assert!(!load_manifest(root).unwrap().files.is_empty());
        forget_baseline(root).unwrap();
        // No record of earlier files means none can be read as deleted on either side.
        assert!(load_manifest(root).unwrap().files.is_empty());
        // Nothing to forget is fine too.
        forget_baseline(root).unwrap();
    }

    #[test]
    fn remote_paths_cannot_leave_the_vault() {
        // Literal and percent-encoded traversal, as a hostile server could list it.
        for href in [
            "/dav/.helixnotes/attachments/..%2F..%2F..%2Fescaped.md",
            "/dav/.helixnotes/attachments/../../../escaped.md",
            "/dav/..%2Fescaped.md",
            "/dav/Notes/./x.md",
            "/dav/Notes//x.md",
            "/dav/Notes%5C..%5Cx.md",
            "/dav/Notes/x%00.md",
        ] {
            let error = href_to_relpath(href, "/dav").unwrap_err();
            assert!(error.starts_with("Sync stopped: "), "{href}: {error}");
        }
        // The folder must end at a path boundary.
        assert!(href_to_relpath("/dav-other/x.md", "/dav").is_err());
        assert_eq!(
            href_to_relpath("/dav/Notes/x.md", "/dav")
                .unwrap()
                .as_deref(),
            Some("Notes/x.md")
        );
        assert_eq!(
            href_to_relpath("/x.md", "/").unwrap().as_deref(),
            Some("x.md")
        );
        // Ordinary names stay valid, including ones only some platforms dislike.
        for rel in [
            "Notes/a b.md",
            "Notes/é ü.md",
            "Notes/A: B.md",
            "Notes/v1.2.md",
            ".helixnotes/attachments/x.png",
        ] {
            assert!(validate_relpath_for(rel, false).is_ok(), "{rel}");
        }
        // Windows: drive prefixes, streams and names it would silently rewrite.
        for rel in [
            "D:x.md",
            "D: draft.md",
            "Notes/x.md:stream",
            "Notes/.. /x.md",
            "Notes/x.md.",
        ] {
            assert!(validate_relpath_for(rel, true).is_err(), "{rel}");
        }
        assert!(validate_relpath_for("Notes/a b.md", true).is_ok());

        // Even a key that slipped past the listing never reaches the disk.
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path().join("vault");
        fs::create_dir_all(&root).unwrap();
        let mut changes = HashMap::new();
        for rel in [
            "../escaped.md",
            ".helixnotes/attachments/../../../escaped.md",
            "/abs.md",
        ] {
            assert!(
                write_local(&root, rel, b"x", &mut changes).is_err(),
                "{rel}"
            );
            assert!(write_local_if_unchanged(
                &tokio::sync::Mutex::new(()),
                &root,
                rel,
                b"x",
                None,
                &mut changes
            )
            .is_err());
        }
        assert!(changes.is_empty());
        assert!(!vault.path().join("escaped.md").exists());
    }

    #[test]
    fn a_listing_with_a_path_outside_the_folder_stops_the_sync() {
        let listing = |href: &str| {
            format!(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"><d:response><d:href>/dav/ok.md</d:href><d:propstat><d:prop><d:getetag>"a"</d:getetag></d:prop></d:propstat></d:response><d:response><d:href>{href}</d:href><d:propstat><d:prop><d:getetag>"b"</d:getetag></d:prop></d:propstat></d:response></d:multistatus>"#
            )
        };
        // One hostile entry fails the whole listing, so nothing from it is acted on.
        for href in [
            "/dav/.helixnotes/attachments/..%2F..%2F..%2FLibrary%2Fx.plist",
            "/dav/../x.md",
            "/elsewhere/x.md",
        ] {
            assert!(parse_multistatus(&listing(href), "/dav").is_err(), "{href}");
        }
        let entries = parse_multistatus(&listing("/dav/Notes/fine.md"), "/dav").unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn other_spellings_of_the_internal_folder_are_never_synced() {
        for rel in [
            ".HelixNotes/history/id/zz-00.md",
            ".HELIXNOTES/trash/x.md",
            ".helixNotes/attachments/x.png",
            ".helixnote\u{17f}/history/x.md",
        ] {
            assert!(!is_synced_relpath(rel), "{rel}");
        }
        assert!(is_synced_relpath(".helixnotes/attachments/x.png"));
        assert!(!is_synced_relpath(".helixnotes/history/id/x.md"));
        assert!(is_synced_relpath("Notes/.HelixNotes.md"));
        assert!(is_synced_relpath("Sub/.HelixNotes/x.md"));
    }

    fn webdav_test_client(
        responses: Vec<(&'static str, &'static str)>,
    ) -> (WebdavClient, std::thread::JoinHandle<Vec<String>>) {
        webdav_test_client_bytes(
            responses
                .into_iter()
                .map(|(status, body)| (status, body.as_bytes().to_vec()))
                .collect(),
        )
    }

    fn webdav_test_client_bytes(
        responses: Vec<(&'static str, Vec<u8>)>,
    ) -> (WebdavClient, std::thread::JoinHandle<Vec<String>>) {
        use std::io::Write;
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/vault", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                let header_end = loop {
                    let len = stream.read(&mut chunk).unwrap();
                    assert!(len > 0);
                    request.extend_from_slice(&chunk[..len]);
                    if let Some(offset) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        break offset + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
                let content_length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|value| value.trim().parse().ok())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + content_length {
                    let len = stream.read(&mut chunk).unwrap();
                    assert!(len > 0);
                    request.extend_from_slice(&chunk[..len]);
                }
                requests.push(headers);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).unwrap();
                stream.write_all(&body).unwrap();
            }
            requests
        });
        (
            WebdavClient::new(WebdavConfig {
                url,
                username: "alice".into(),
                password: "secret".into(),
                listing_parallelism: 0,
            })
            .unwrap(),
            worker,
        )
    }

    #[test]
    fn corrupt_manifest_and_unreadable_vault_stop_without_treating_files_as_deleted() {
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path().to_str().unwrap();
        fs::create_dir_all(helixnotes_dir(root)).unwrap();
        fs::write(manifest_path(root), b"{invalid").unwrap();
        assert!(load_manifest(root)
            .err()
            .unwrap()
            .contains("Invalid sync manifest"));
        fs::write(manifest_path(root), b"{\"files\":{}}").unwrap();
        assert!(scan_local(&vault.path().join("missing").to_string_lossy()).is_err());
        fs::write(vault.path().join("note.md"), b"local").unwrap();
        let old = load_manifest(root).unwrap();
        let rebased = baseline_for_endpoint(old, "new account");
        assert!(rebased.files.is_empty());
        fs::remove_dir_all(helixnotes_dir(root)).unwrap();
        fs::write(helixnotes_dir(root), b"not a directory").unwrap();
        assert!(save_manifest(root, &rebased).is_err());
        assert_eq!(fs::read(vault.path().join("note.md")).unwrap(), b"local");
    }

    #[test]
    fn endpoint_identity_rebases_without_credentials_or_deletion_history() {
        let first = WebdavClient::new(WebdavConfig {
            url: "HTTPS://Example.org:443/team/".into(),
            username: "alice".into(),
            password: "one".into(),
            listing_parallelism: 0,
        })
        .unwrap();
        let same = WebdavClient::new(WebdavConfig {
            url: "https://example.org/team".into(),
            username: "alice".into(),
            password: "other".into(),
            listing_parallelism: 0,
        })
        .unwrap();
        let other = WebdavClient::new(WebdavConfig {
            url: "https://example.org/team".into(),
            username: "bob".into(),
            password: "one".into(),
            listing_parallelism: 0,
        })
        .unwrap();
        let different_folder = WebdavClient::new(WebdavConfig {
            url: "https://example.org/another".into(),
            username: "alice".into(),
            password: "one".into(),
            listing_parallelism: 0,
        })
        .unwrap();
        assert_eq!(first.identity(), same.identity());
        assert_ne!(same.identity(), other.identity());
        assert_ne!(same.identity(), different_folder.identity());
        let manifest = SyncManifest {
            identity: first.identity(),
            files: HashMap::from([(
                "note.md".into(),
                manifest_entry(sha256_hex(b"old"), "\"old\""),
            )]),
        };
        let vault = tempfile::tempdir().unwrap();
        let root = vault.path().to_str().unwrap();
        save_manifest(root, &manifest).unwrap();
        let persisted = fs::read_to_string(manifest_path(root)).unwrap();
        assert!(!persisted.contains("one") && !persisted.contains("alice"));
        assert_eq!(load_manifest(root).unwrap().identity, same.identity());
        assert_eq!(
            baseline_for_endpoint(manifest, &other.identity())
                .files
                .len(),
            0
        );
        assert!(
            baseline_for_endpoint(load_manifest(root).unwrap(), &different_folder.identity())
                .files
                .is_empty()
        );
    }

    #[test]
    fn etagless_remote_content_change_refreshes_local_and_refuses_remote_deletion() {
        let vault = tempfile::tempdir().unwrap();
        fs::write(vault.path().join("note.md"), b"old").unwrap();
        let root = vault.path().to_str().unwrap();
        let local = scan_local(root).unwrap();
        let manifest = SyncManifest {
            identity: String::new(),
            files: HashMap::from([("note.md".into(), manifest_entry(sha256_hex(b"old"), ""))]),
        };
        let (client, requests) = webdav_test_client(vec![("200 OK", "new")]);
        let mut changes = HashMap::new();
        let (next, summary) = apply_changes(
            &client,
            root,
            &local,
            &HashMap::from([("note.md".into(), String::new())]),
            &manifest,
            &mut changes,
            &tokio::sync::Mutex::new(()),
        )
        .unwrap();
        assert_eq!(summary.downloaded, 1);
        assert_eq!(fs::read(vault.path().join("note.md")).unwrap(), b"new");
        assert_eq!(next.files["note.md"].remote_hash, sha256_hex(b"new"));
        assert!(requests.join().unwrap()[0].starts_with("GET /vault/note.md "));
        let (client, requests) = webdav_test_client(vec![("200 OK", "new")]);
        let error = apply_changes(
            &client,
            root,
            &HashMap::new(),
            &HashMap::from([("note.md".into(), String::new())]),
            &next,
            &mut HashMap::new(),
            &tokio::sync::Mutex::new(()),
        )
        .err()
        .unwrap();
        assert!(error.contains("Cannot safely delete"));
        assert_eq!(requests.join().unwrap().len(), 1); // GET only, never DELETE
    }

    // Some servers hand out a new ETag for bytes that did not change. That is not a remote
    // edit, so a local edit must upload instead of becoming a conflict copy.
    #[test]
    fn a_new_etag_for_the_same_remote_bytes_is_not_a_remote_change() {
        let vault = tempfile::tempdir().unwrap();
        fs::write(vault.path().join("note.md"), b"mine").unwrap();
        let root = vault.path().to_str().unwrap();
        let manifest = SyncManifest {
            identity: String::new(),
            files: HashMap::from([(
                "note.md".into(),
                manifest_entry(sha256_hex(b"old"), "\"e1\""),
            )]),
        };
        let remote = HashMap::from([("note.md".to_string(), "\"e2\"".to_string())]);
        // Every download is followed by a PROPFIND that rechecks the ETag.
        let e2 = r#"<multistatus><response><href>/vault/note.md</href><getetag>&quot;e2&quot;</getetag></response></multistatus>"#;
        let e3 = r#"<multistatus><response><href>/vault/note.md</href><getetag>&quot;e3&quot;</getetag></response></multistatus>"#;
        let (client, requests) = webdav_test_client(vec![
            ("200 OK\r\nETag: \"e2\"", "old"),
            ("207 Multi-Status", e2),
            ("204 No Content", ""),
            ("207 Multi-Status", e3),
            ("200 OK\r\nETag: \"e3\"", "mine"),
            ("207 Multi-Status", e3),
        ]);
        let (next, summary) = apply_changes(
            &client,
            root,
            &scan_local(root).unwrap(),
            &remote,
            &manifest,
            &mut HashMap::new(),
            &tokio::sync::Mutex::new(()),
        )
        .unwrap();
        assert_eq!((summary.conflicts, summary.uploaded), (0, 1));
        assert_eq!(fs::read(vault.path().join("note.md")).unwrap(), b"mine");
        assert_eq!(fs::read_dir(vault.path()).unwrap().count(), 1);
        assert_eq!(next.files["note.md"].remote_etag, "\"e3\"");
        let requests = requests.join().unwrap();
        assert!(requests[2].starts_with("PUT /vault/note.md "));
        assert!(requests[2]
            .to_ascii_lowercase()
            .contains("if-match: \"e2\""));

        // An untouched note only records the new ETag.
        fs::write(vault.path().join("note.md"), b"old").unwrap();
        let (client, requests) = webdav_test_client(vec![
            ("200 OK\r\nETag: \"e2\"", "old"),
            ("207 Multi-Status", e2),
        ]);
        let (next, summary) = apply_changes(
            &client,
            root,
            &scan_local(root).unwrap(),
            &remote,
            &manifest,
            &mut HashMap::new(),
            &tokio::sync::Mutex::new(()),
        )
        .unwrap();
        assert_eq!(
            (summary.conflicts, summary.uploaded, summary.downloaded),
            (0, 0, 0)
        );
        assert_eq!(next.files["note.md"].remote_etag, "\"e2\"");
        assert_eq!(requests.join().unwrap().len(), 2);

        // A note deleted locally is deleted remotely instead of coming back.
        fs::remove_file(vault.path().join("note.md")).unwrap();
        let (client, requests) = webdav_test_client(vec![
            ("200 OK\r\nETag: \"e2\"", "old"),
            ("207 Multi-Status", e2),
            ("204 No Content", ""),
        ]);
        let (_, summary) = apply_changes(
            &client,
            root,
            &HashMap::new(),
            &remote,
            &manifest,
            &mut HashMap::new(),
            &tokio::sync::Mutex::new(()),
        )
        .unwrap();
        assert_eq!((summary.deleted_remote, summary.downloaded), (1, 0));
        assert!(!vault.path().join("note.md").exists());
        assert!(requests.join().unwrap()[2].starts_with("DELETE /vault/note.md "));
    }

    #[test]
    fn changed_get_etag_never_installs_stale_snapshot() {
        let (client, requests) = webdav_test_client(vec![("200 OK\r\nETag: \"new\"", "newer")]);
        assert!(client
            .get("note.md", "\"old\"")
            .unwrap_err()
            .contains("retry"));
        let requests = requests.join().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("if-match: \"old\""));
    }

    #[test]
    fn conditional_writes_preserve_concurrent_remote_content() {
        let (client, requests) = webdav_test_client(vec![("412 Precondition Failed", "")]);
        assert!(client
            .put("note.md", b"edited", Some("\"old\""))
            .unwrap_err()
            .contains("retry"));
        let requests = requests.join().unwrap();
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("if-match: \"old\""));

        let (client, requests) = webdav_test_client(vec![("412 Precondition Failed", "")]);
        assert!(client
            .delete("note.md", "\"old\"")
            .unwrap_err()
            .contains("retry"));
        assert!(requests.join().unwrap()[0]
            .to_ascii_lowercase()
            .contains("if-match: \"old\""));

        let (client, requests) = webdav_test_client(vec![("412 Precondition Failed", "")]);
        assert!(client
            .put("new.md", b"new", None)
            .unwrap_err()
            .contains("retry"));
        assert!(requests.join().unwrap()[0]
            .to_ascii_lowercase()
            .contains("if-none-match: *"));
        let (client, requests) = webdav_test_client(vec![]);
        assert!(client
            .put("note.md", b"edited", Some(""))
            .unwrap_err()
            .contains("no strong ETag"));
        assert!(client
            .delete("note.md", "")
            .unwrap_err()
            .contains("no strong ETag"));
        assert!(requests.join().unwrap().is_empty());
    }

    #[test]
    fn multistatus_preserves_named_and_numeric_references() {
        let xml = r#"<d:multistatus xmlns:d="DAV:"><d:response>
            <d:href>/vault/a&amp;b&apos;&lt;&gt;&quot;-&#233;-&#x96EA;.md</d:href>
            <d:propstat><d:prop>
                <d:getetag>&quot;v&amp;&apos;&lt;&gt;&#x96EA;&#233;&quot;</d:getetag>
            </d:prop></d:propstat>
        </d:response></d:multistatus>"#;
        let entries = parse_multistatus(xml, "/vault").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relpath, "a&b'<>\"-é-雪.md");
        assert_eq!(entries[0].etag, "\"v&'<>雪é\"");
        assert!(!entries[0].is_dir);
    }

    #[test]
    fn multistatus_preserves_mixed_cdata_and_strong_etag_policy() {
        let xml = r#"<multistatus xmlns="DAV:">
            <response>
                <href>/vault/<![CDATA[café&amp;]]>&#x96EA;.md</href>
                <propstat><prop><getetag>&#34;<![CDATA[v&amp;]]>&#x96EA;&#x22;</getetag></prop></propstat>
            </response>
            <response>
                <href><![CDATA[/vault/folder/]]></href>
                <propstat><prop><getetag><![CDATA[W/"weak"]]></getetag>
                <resourcetype><collection/></resourcetype></prop></propstat>
            </response>
        </multistatus>"#;
        let entries = parse_multistatus(xml, "/vault").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].relpath, "café&amp;雪.md");
        assert_eq!(entries[0].etag, "\"v&amp;雪\"");
        assert_eq!(entries[1].relpath, "folder");
        assert!(entries[1].etag.is_empty());
        assert!(entries[1].is_dir);
    }

    #[test]
    fn multistatus_rejects_unknown_and_malformed_captured_references() {
        for reference in [
            "&unknown;",
            "&;",
            "&#;",
            "&#xZZ;",
            "&#xD800;",
            "&#x110000;",
            "&#0;",
            "&amp",
            "&#123",
        ] {
            for field in ["href", "getetag"] {
                let (href, etag) = if field == "href" {
                    (format!("/vault/bad{reference}.md"), "\"ok\"".to_string())
                } else {
                    ("/vault/bad.md".to_string(), format!("\"bad{reference}\""))
                };
                let xml = format!(
                    "<multistatus><response><href>/vault/good.md</href>\
                    <getetag>\"good\"</getetag></response><response>\
                    <href>{href}</href><getetag>{etag}</getetag>\
                    </response></multistatus>"
                );
                assert!(
                    parse_multistatus(&xml, "/vault").is_err(),
                    "accepted {reference:?} in {field}"
                );
            }
        }
    }

    #[test]
    fn multistatus_does_not_expand_custom_or_external_dtd_entities() {
        for declaration in [
            r#"<!ENTITY custom "expanded">"#,
            r#"<!ENTITY custom SYSTEM "file:///nonexistent-helixnotes-entity">"#,
        ] {
            let xml = format!(
                "<!DOCTYPE multistatus [{declaration}]><multistatus><response>\
                <href>/vault/&custom;.md</href><getetag>\"etag\"</getetag>\
                </response></multistatus>"
            );
            assert!(parse_multistatus(&xml, "/vault")
                .err()
                .unwrap()
                .contains("Unknown XML entity"));
        }
    }

    #[test]
    fn get_preserves_binary_download_and_rechecks_strong_etag() {
        let bytes: Vec<u8> = (0..HASH_BUFFER_SIZE * 2 + 137)
            .map(|i| (i % 256) as u8)
            .collect();
        let listing = br#"<d:multistatus xmlns:d="DAV:"><d:response>
            <d:href>/vault/file.bin</d:href>
            <d:propstat><d:prop><d:getetag>&quot;stable&quot;</d:getetag></d:prop></d:propstat>
        </d:response></d:multistatus>"#;
        let (client, requests) = webdav_test_client_bytes(vec![
            ("200 OK\r\nETag: \"stable\"", bytes.clone()),
            ("207 Multi-Status", listing.to_vec()),
        ]);
        assert_eq!(client.get("file.bin", "\"stable\"").unwrap(), bytes);
        let requests = requests.join().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("GET /vault/file.bin "));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("if-match: \"stable\""));
        assert!(requests[1].starts_with("PROPFIND /vault/file.bin "));
        assert!(requests[1].to_ascii_lowercase().contains("depth: 0"));
    }

    #[test]
    fn get_rejects_etag_changed_after_download_and_http_failures() {
        let listing = r#"<multistatus><response><href>/vault/note.md</href>
            <getetag>&quot;new&quot;</getetag></response></multistatus>"#;
        let (client, requests) = webdav_test_client(vec![
            ("200 OK\r\nETag: \"old\"", "snapshot"),
            ("207 Multi-Status", listing),
        ]);
        assert!(client
            .get("note.md", "\"old\"")
            .unwrap_err()
            .contains("retry"));
        assert_eq!(requests.join().unwrap().len(), 2);

        for (status, expected_error) in [
            ("412 Precondition Failed", "retry"),
            ("404 Not Found", "HTTP 404"),
        ] {
            let (client, requests) = webdav_test_client(vec![(status, "ignored")]);
            assert!(client
                .get("note.md", "\"old\"")
                .unwrap_err()
                .contains(expected_error));
            assert_eq!(requests.join().unwrap().len(), 1);
        }
    }

    #[test]
    fn get_enforces_whole_body_timeout_for_progressing_chunked_download() {
        use std::io::Write;
        use std::net::TcpListener;
        use std::time::Duration;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/vault", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let count = stream.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            assert!(request.starts_with(b"GET /vault/file.bin "));
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\
                    Connection: close\r\n\r\n1\r\nx\r\n",
                )
                .unwrap();
            let mut chunks_sent = 1;
            // Each gap is below the 400ms timeout, but the full body takes 1s.
            for _ in 0..20 {
                std::thread::sleep(Duration::from_millis(50));
                if stream.write_all(b"1\r\nx\r\n").is_err() {
                    return chunks_sent;
                }
                chunks_sent += 1;
            }
            let _ = stream.write_all(b"0\r\n\r\n");
            chunks_sent
        });
        let mut client = WebdavClient::new(WebdavConfig {
            url,
            username: String::new(),
            password: String::new(),
            listing_parallelism: 0,
        })
        .unwrap();
        client.client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(400))
            .build()
            .unwrap();

        let result = client.get("file.bin", "");
        let chunks_sent = server.join().unwrap();
        assert!(
            chunks_sent > 1,
            "server must make progress during the download"
        );
        assert!(
            result.is_err(),
            "accepted a body exceeding the total timeout"
        );
    }
}
