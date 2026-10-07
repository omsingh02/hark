use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tokio::io::AsyncWriteExt;

/// Default maximum total disk cache size (100 MiB) before LRU eviction triggers
const DEFAULT_MAX_CACHE_BYTES: u64 = 100 * 1024 * 1024;

/// Maximum allowed image payload (5 MiB) to prevent decompression or memory exhaustion attacks
const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;

/// Maximum duration allowed for HTTP cover art downloads per Rule P-4
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Target resolution token for high-DPI Apple Music artwork (800x800)
const HIGH_RES_APPLE_DIMS: &str = "800x800";

/// High-DPI JPEG suffix replacement for Apple CDN artwork
const HIGH_RES_APPLE_SUFFIX: &str = "800x800bb.jpg";

/// Errors occurring during cover art caching and retrieval
#[derive(Debug)]
pub enum CacheError {
    /// Filesystem input/output failure
    Io(std::io::Error),
    /// Network connection or transport error
    Network(reqwest::Error),
    /// HTTP non-success response code
    HttpStatus(reqwest::StatusCode),
    /// Empty or non-HTTP/HTTPS URL
    InvalidUrl,
    /// Blank or dummy placeholder key
    InvalidKey,
    /// Content magic bytes do not match supported image formats (JPEG/PNG/WEBP)
    InvalidImageFormat,
    /// Image payload exceeds maximum safety threshold
    PayloadTooLarge(u64),
    /// A download for this key is already actively streaming
    AlreadyInFlight,
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {}", e),
            Self::Network(e) => write!(f, "Network error: {}", e),
            Self::HttpStatus(s) => write!(f, "HTTP error: status {}", s),
            Self::InvalidUrl => write!(f, "Invalid URL: must begin with http:// or https://"),
            Self::InvalidKey => write!(f, "Invalid key: cannot be blank or dummy placeholder"),
            Self::InvalidImageFormat => write!(f, "Invalid image format: magic bytes mismatch"),
            Self::PayloadTooLarge(n) => write!(f, "Image too large (> {} bytes)", n),
            Self::AlreadyInFlight => write!(f, "Download already in flight"),
        }
    }
}

impl std::error::Error for CacheError {}

impl From<std::io::Error> for CacheError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<reqwest::Error> for CacheError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network(e)
    }
}

pub struct CoverCacheManager {
    cache_dir: PathBuf,
    tmp_dir: PathBuf,
    max_bytes: u64,
    in_flight: Mutex<HashSet<String>>,
}

impl CoverCacheManager {
    /// Creates a new CoverCacheManager targeting `cache_dir` with a maximum byte quota
    pub fn new(cache_dir: PathBuf, max_bytes: u64) -> Result<Self, std::io::Error> {
        let tmp_dir = cache_dir.join(".tmp");
        fs::create_dir_all(&tmp_dir)?;

        Ok(Self {
            cache_dir,
            tmp_dir,
            max_bytes: if max_bytes == 0 { DEFAULT_MAX_CACHE_BYTES } else { max_bytes },
            in_flight: Mutex::new(HashSet::new()),
        })
    }

    /// Resolves the default XDG cache directory: ~/.cache/hark/covers
    pub fn default_cache_dir() -> PathBuf {
        let base = std::env::var("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
                PathBuf::from(home).join(".cache")
            });
        base.join("hark").join("covers")
    }

    /// Default instance using standard XDG cache directory and 100MB limit
    pub fn default_manager() -> Result<Self, std::io::Error> {
        Self::new(Self::default_cache_dir(), DEFAULT_MAX_CACHE_BYTES)
    }

    /// Sanitizes key to strictly alphanumeric, or falls back to a deterministic 16-hex hash
    pub fn sanitize_key(key: &str) -> String {
        let trimmed = key.trim();
        if !trimmed.is_empty()
            && trimmed.len() <= 64
            && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            trimmed.to_string()
        } else {
            let crc = crc32fast::hash(trimmed.as_bytes());
            format!("hash_{:08x}_{}", crc, trimmed.len())
        }
    }

    /// Upgrades Apple CDN image URLs to high-DPI quality (800x800), supporting both
    /// legacy static dimensions and modern dynamic `{w}x{h}` templates.
    pub fn optimize_apple_url(url: &str) -> String {
        // Only modify URLs hosted on Apple CDN domains
        if !url.contains("mzstatic.com") && !url.contains("apple.com") {
            return url.to_string();
        }

        // 1. Dynamic template replacement (e.g. {w}x{h}bb.jpg -> 800x800bb.jpg)
        if url.contains("{w}x{h}") {
            return url.replace("{w}x{h}", HIGH_RES_APPLE_DIMS);
        }

        // 2. Fixed dimension replacement
        if url.contains("400x400cc.jpg") {
            url.replace("400x400cc.jpg", HIGH_RES_APPLE_SUFFIX)
        } else if url.contains("400x400bb.jpg") {
            url.replace("400x400bb.jpg", HIGH_RES_APPLE_SUFFIX)
        } else {
            url.to_string()
        }
    }

    /// Checks if a cover is cached and valid on disk. Returns `None` immediately if `key` is blank or dummy `"0"`.
    pub fn get_local_path(&self, key: &str) -> Option<PathBuf> {
        let trimmed = key.trim();
        if trimmed.is_empty() || trimmed == "0" {
            return None;
        }

        let filename = format!("{}.jpg", Self::sanitize_key(trimmed));
        let path = self.cache_dir.join(filename);
        if path.is_file() {
            if let Ok(meta) = path.metadata() {
                if meta.len() > 0 {
                    return Some(path);
                }
            }
        }
        None
    }

    /// Returns a `file:///...` URI if the cover is locally cached. Returns `None` if `key` is blank or dummy `"0"`.
    pub fn get_local_uri(&self, key: &str) -> Option<String> {
        self.get_local_path(key)
            .map(|p| format!("file://{}", p.display()))
    }

    /// Validates magic bytes for JPEG, PNG, or WEBP images
    pub fn validate_image_magic(bytes: &[u8]) -> bool {
        if bytes.len() < 4 {
            return false;
        }
        // JPEG: FF D8 FF
        if bytes[0] == 0xFF && bytes[1] == 0xD8 && bytes[2] == 0xFF {
            return true;
        }
        // PNG: 89 50 4E 47 (\x89PNG)
        if bytes[0] == 0x89 && bytes[1] == b'P' && bytes[2] == b'N' && bytes[3] == b'G' {
            return true;
        }
        // WEBP: RIFF....WEBP
        if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            return true;
        }
        false
    }

    /// Streams an image to temporary file, validates magic bytes, and atomically renames.
    /// Deduplicates concurrent downloads for the same key. Quota eviction is offloaded to a blocking thread.
    pub async fn ensure_cached(
        &self,
        client: &reqwest::Client,
        raw_url: &str,
        key: &str,
    ) -> Result<PathBuf, CacheError> {
        let trimmed_key = key.trim();
        if trimmed_key.is_empty() || trimmed_key == "0" {
            return Err(CacheError::InvalidKey);
        }

        let trimmed_url = raw_url.trim();
        if !trimmed_url.starts_with("http://") && !trimmed_url.starts_with("https://") {
            return Err(CacheError::InvalidUrl);
        }

        let clean_key = Self::sanitize_key(trimmed_key);
        let dest_path = self.cache_dir.join(format!("{}.jpg", clean_key));

        // 1. Return immediately if already cached
        if dest_path.is_file() {
            if let Ok(meta) = dest_path.metadata() {
                if meta.len() > 0 {
                    return Ok(dest_path);
                }
            }
        }

        // 2. Single-flight deduplication
        {
            let mut guard = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
            if guard.contains(&clean_key) {
                return Err(CacheError::AlreadyInFlight);
            }
            guard.insert(clean_key.clone());
        }

        // RAII guard to always remove from in_flight on return
        struct FlightGuard<'a> {
            in_flight: &'a Mutex<HashSet<String>>,
            key: String,
        }
        impl<'a> Drop for FlightGuard<'a> {
            fn drop(&mut self) {
                let mut guard = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
                guard.remove(&self.key);
            }
        }
        let _guard = FlightGuard {
            in_flight: &self.in_flight,
            key: clean_key.clone(),
        };

        // 3. Optimize URL (e.g. Apple 400 -> 800 or template expansion)
        let download_url = Self::optimize_apple_url(trimmed_url);

        // 4. Atomic staging path
        let ts = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let tmp_path = self.tmp_dir.join(format!("{}_{}.tmp", clean_key, ts));

        // 5. Stream request (F-2, P-4)
        let mut response = client
            .get(&download_url)
            .timeout(REQUEST_TIMEOUT)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(CacheError::HttpStatus(response.status()));
        }

        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)
            .await?;

        let mut total_bytes = 0u64;
        let mut first_chunk = true;

        while let Some(chunk) = response.chunk().await? {
            total_bytes += chunk.len() as u64;
            if total_bytes > MAX_IMAGE_BYTES {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                return Err(CacheError::PayloadTooLarge(total_bytes));
            }

            if first_chunk {
                if !Self::validate_image_magic(&chunk) {
                    let _ = tokio::fs::remove_file(&tmp_path).await;
                    return Err(CacheError::InvalidImageFormat);
                }
                first_chunk = false;
            }

            file.write_all(&chunk).await?;
        }

        file.flush().await?;
        drop(file);

        if total_bytes == 0 {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(CacheError::InvalidImageFormat);
        }

        // 6. Atomic rename (F-1)
        tokio::fs::rename(&tmp_path, &dest_path).await?;

        // 7. Enforce quota asynchronously on blocking thread pool (Rule A-1)
        let cache_dir_clone = self.cache_dir.clone();
        let tmp_dir_clone = self.tmp_dir.clone();
        let max_bytes_val = self.max_bytes;
        tokio::task::spawn_blocking(move || {
            let _ = Self::prune_cache_dir(&cache_dir_clone, &tmp_dir_clone, max_bytes_val);
        });

        Ok(dest_path)
    }

    /// Synchronous quota check convenience wrapper
    #[allow(dead_code)]
    pub fn enforce_quota(&self) -> Result<(), std::io::Error> {
        Self::prune_cache_dir(&self.cache_dir, &self.tmp_dir, self.max_bytes)
    }

    /// Prunes cache directory when it exceeds max_bytes, keeping newest files (LRU by mtime)
    pub fn prune_cache_dir(cache_dir: &Path, tmp_dir: &Path, max_bytes: u64) -> Result<(), std::io::Error> {
        let entries = match fs::read_dir(cache_dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
        let mut total_size = 0u64;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(meta) = path.metadata() {
                    let len = meta.len();
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    total_size += len;
                    files.push((path, len, mtime));
                }
            }
        }

        if total_size > max_bytes {
            // Sort ascending by modification time (oldest first)
            files.sort_by_key(|(_, _, mtime)| *mtime);

            let target_size = (max_bytes * 80) / 100; // Drop to 80% quota
            let mut current_size = total_size;

            for (path, len, _) in files {
                if current_size <= target_size {
                    break;
                }
                if fs::remove_file(&path).is_ok() {
                    current_size = current_size.saturating_sub(len);
                }
            }
        }

        // Also clean up any abandoned .tmp files older than 30 minutes
        if let Ok(tmp_entries) = fs::read_dir(tmp_dir) {
            let now = SystemTime::now();
            for entry in tmp_entries.flatten() {
                let path = entry.path();
                if let Ok(meta) = path.metadata() {
                    if let Ok(mtime) = meta.modified() {
                        if let Ok(age) = now.duration_since(mtime) {
                            if age > Duration::from_secs(1800) {
                                let _ = fs::remove_file(path);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_sanitize_key() {
        assert_eq!(CoverCacheManager::sanitize_key("837534255"), "837534255");
        assert_eq!(CoverCacheManager::sanitize_key("track_123-abc"), "track_123-abc");

        // Path traversal attempts must be hashed safely
        let unsafe_key = "../../etc/passwd";
        let safe = CoverCacheManager::sanitize_key(unsafe_key);
        assert!(!safe.contains('/'));
        assert!(!safe.contains(".."));
        assert!(safe.starts_with("hash_"));
    }

    #[test]
    fn test_empty_and_dummy_zero_keys() {
        let temp_dir = std::env::temp_dir().join(format!("shazam_key_test_{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let manager = CoverCacheManager::new(temp_dir.clone(), 1000).expect("Failed to create manager");

        // Blank and "0" must return None immediately
        assert!(manager.get_local_path("").is_none());
        assert!(manager.get_local_path("   ").is_none());
        assert!(manager.get_local_path("0").is_none());
        assert!(manager.get_local_uri("0").is_none());

        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_optimize_apple_url() {
        // Standard Apple CDN static dimensions
        let input = "https://is1-ssl.mzstatic.com/image/thumb/Music/cover.jpg/400x400cc.jpg";
        let expected = "https://is1-ssl.mzstatic.com/image/thumb/Music/cover.jpg/800x800bb.jpg";
        assert_eq!(CoverCacheManager::optimize_apple_url(input), expected);

        let input_bb = "https://is1-ssl.mzstatic.com/image/thumb/Music/cover.jpg/400x400bb.jpg";
        assert_eq!(CoverCacheManager::optimize_apple_url(input_bb), expected);

        // Apple modern dynamic template URLs
        let input_template = "https://is3-ssl.mzstatic.com/image/thumb/Music/{w}x{h}bb.jpg";
        let expected_template = "https://is3-ssl.mzstatic.com/image/thumb/Music/800x800bb.jpg";
        assert_eq!(CoverCacheManager::optimize_apple_url(input_template), expected_template);

        // Non-Apple URLs must NOT be modified
        let non_apple = "https://example.com/images/400x400cc.jpg";
        assert_eq!(CoverCacheManager::optimize_apple_url(non_apple), non_apple);
    }

    #[test]
    fn test_validate_image_magic() {
        // Valid JPEG
        assert!(CoverCacheManager::validate_image_magic(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00]));
        // Valid PNG
        assert!(CoverCacheManager::validate_image_magic(&[0x89, b'P', b'N', b'G', 0x0D]));
        // Valid WEBP
        assert!(CoverCacheManager::validate_image_magic(b"RIFF\x00\x00\x00\x00WEBPVP8 "));
        // Invalid HTML or random text
        assert!(!CoverCacheManager::validate_image_magic(b"<!DOCTYPE html><html>"));
        assert!(!CoverCacheManager::validate_image_magic(b"{\"error\": \"not found\"}"));
        assert!(!CoverCacheManager::validate_image_magic(&[]));
    }

    #[test]
    fn test_cache_and_quota_eviction() {
        let temp_dir = std::env::temp_dir().join(format!("hark_cache_test_{}", SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        let manager = CoverCacheManager::new(temp_dir.clone(), 1000).expect("Failed to create manager");

        // Write a mock valid JPEG (size: 400 bytes)
        let file1 = temp_dir.join("song1.jpg");
        let mut f1 = fs::File::create(&file1).unwrap();
        f1.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        f1.write_all(&vec![0u8; 396]).unwrap();
        drop(f1);

        assert!(manager.get_local_path("song1").is_some());
        assert!(manager.get_local_uri("song1").unwrap().starts_with("file://"));

        // Write a second mock JPEG (size: 400 bytes)
        let file2 = temp_dir.join("song2.jpg");
        let mut f2 = fs::File::create(&file2).unwrap();
        f2.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        f2.write_all(&vec![0u8; 396]).unwrap();
        drop(f2);

        // Write a third mock JPEG (size: 400 bytes) -> Total 1200 bytes > max_bytes 1000
        let file3 = temp_dir.join("song3.jpg");
        let mut f3 = fs::File::create(&file3).unwrap();
        f3.write_all(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        f3.write_all(&vec![0u8; 396]).unwrap();
        drop(f3);

        manager.enforce_quota().unwrap();

        // Total size should now be <= 800 bytes (80% of 1000)
        let remaining_files = fs::read_dir(&temp_dir)
            .unwrap()
            .flatten()
            .filter(|e| e.path().is_file())
            .count();
        assert!(remaining_files <= 2);

        // Cleanup
        let _ = fs::remove_dir_all(temp_dir);
    }
}
