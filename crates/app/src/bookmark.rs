//! Security-scoped bookmarks: how the sandboxed (Mac App Store) build keeps read access
//! to the vault between launches.
//!
//! In the App Sandbox the app can read a folder only after the user picks it in an open
//! panel, and only until it quits. Picking the vault saves a read-only, security-scoped
//! bookmark next to the config (`vault.bookmark`); at launch the bookmark is resolved and
//! `startAccessingSecurityScopedResource` reopens the folder, which covers scanning, the
//! FSEvents watcher and the Obsidian plugin's `state.json` inside the vault.
//!
//! The direct (Developer ID) build is not sandboxed and doesn't use any of this:
//! [`sandboxed`] is false there and the vault is read by path, as before.

use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_foundation::{NSData, NSURL, NSURLBookmarkCreationOptions, NSURLBookmarkResolutionOptions};

/// Whether this process runs in the App Sandbox (macOS sets this for every sandboxed app).
pub fn sandboxed() -> bool {
    std::env::var_os("APP_SANDBOX_CONTAINER_ID").is_some()
}

/// Where the vault's bookmark lives: next to the config file.
pub fn file_for(config: &Path) -> PathBuf {
    config.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new(".")).join("vault.bookmark")
}

/// Saves a read-only, security-scoped bookmark to `dir` in `file`. Call it while the
/// process has access to `dir` (right after the open panel returned it).
pub fn save(file: &Path, dir: &Path) -> Result<(), String> {
    let url = NSURL::from_directory_path(dir).ok_or_else(|| format!("not a path: {}", dir.display()))?;
    let options = NSURLBookmarkCreationOptions::WithSecurityScope | NSURLBookmarkCreationOptions::SecurityScopeAllowOnlyReadAccess;
    let data = url
        .bookmarkDataWithOptions_includingResourceValuesForKeys_relativeToURL_error(options, None, None)
        .map_err(|e| e.localizedDescription().to_string())?;
    write(file, &data.to_vec())
}

fn write(file: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(d) = file.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("bookmark.tmp");
    std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, file)).map_err(|e| e.to_string())
}

/// Open access to a bookmarked folder. Dropping it gives the access back, so keep it as
/// long as anything reads or watches the folder.
pub struct Access {
    url: Retained<NSURL>,
    started: bool,
}

impl Drop for Access {
    fn drop(&mut self) {
        if self.started {
            unsafe { self.url.stopAccessingSecurityScopedResource() };
        }
    }
}

/// Resolves the bookmark in `file` and starts accessing it, if it still points at `want`.
/// A stale bookmark (the folder was moved and back, or the volume renamed) is renewed.
pub fn open(file: &Path, want: &Path) -> Result<Access, String> {
    let bytes = std::fs::read(file).map_err(|e| format!("no bookmark ({}): {e}", file.display()))?;
    let data = NSData::with_bytes(&bytes);
    let mut stale = Bool::NO;
    let url = unsafe {
        NSURL::URLByResolvingBookmarkData_options_relativeToURL_bookmarkDataIsStale_error(
            &data,
            NSURLBookmarkResolutionOptions::WithSecurityScope | NSURLBookmarkResolutionOptions::WithoutUI,
            None,
            &mut stale,
        )
    }
    .map_err(|e| format!("bookmark doesn't resolve: {}", e.localizedDescription()))?;
    let got = url.to_file_path().ok_or("bookmark isn't a file URL")?;
    if !same_dir(&got, want) {
        return Err(format!("bookmark is for {}, not {}", got.display(), want.display()));
    }
    let started = unsafe { url.startAccessingSecurityScopedResource() };
    if !started && sandboxed() {
        return Err(format!("macOS refused access to {}", got.display()));
    }
    let access = Access { url, started };
    if stale.as_bool()
        && let Err(e) = save(file, &got)
    {
        eprintln!("vault bookmark: could not renew a stale bookmark: {e}");
    }
    Ok(access)
}

/// Paths equal up to a trailing slash.
fn same_dir(a: &Path, b: &Path) -> bool {
    a.components().eq(b.components())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bookmark_round_trip() {
        let dir = std::env::temp_dir().join(format!("vw-bookmark-{}", std::process::id()));
        let vault = dir.join("My Vault");
        std::fs::create_dir_all(&vault).unwrap();
        let vault = vault.canonicalize().unwrap();
        let file = file_for(&dir.join("config.json"));
        save(&file, &vault).unwrap();
        let access = open(&file, &vault).unwrap();
        assert!(same_dir(&access.url.to_file_path().unwrap(), &vault));
        drop(access);
        let other = dir.join("Other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(open(&file, &other.canonicalize().unwrap()).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_bookmark_is_an_error() {
        assert!(open(Path::new("/nonexistent/vault.bookmark"), Path::new("/v")).is_err());
    }

    #[test]
    fn trailing_slash_is_the_same_dir() {
        assert!(same_dir(Path::new("/a/b/"), Path::new("/a/b")));
        assert!(!same_dir(Path::new("/a/b"), Path::new("/a/c")));
    }
}
