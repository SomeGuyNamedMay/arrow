//! Compiled plugins kept on disk, so starting Arrow doesn't compile them again.
//!
//! Turning a plugin's WebAssembly into machine code (wasmtime's Cranelift
//! compiler) was nearly all of Arrow's startup time, and it happened on every
//! launch. Now the result is saved the first time, and later launches load it
//! back, which takes milliseconds:
//!
//! 1. The cache key is a hash of the plugin's bytes together with the engine's
//!    compatibility hash, which changes with the wasmtime version and with
//!    any engine setting that affects the machine code. So a rebuilt plugin,
//!    an upgraded wasmtime or a changed `Config` simply misses the cache.
//! 2. On a hit, the file is mapped back in with `Component::deserialize_file`.
//!    If wasmtime rejects it (an older format, a damaged file), we fall back
//!    to compiling.
//! 3. On a miss, the plugin is compiled as before, then serialized and written
//!    to the cache. The write goes to a temporary file that is then renamed,
//!    so another Arrow starting at the same moment never reads half a file.
//!
//! The cache lives in the user's local cache folder (see
//! [`crate::config::user_cache_dir`]). Deleting it is always safe. Old
//! entries aren't cleaned up automatically, but each is only as big as one
//! compiled plugin.

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use wasmtime::component::Component;
use wasmtime::Engine;

/// Compile `bytes` into a component, or load the copy compiled earlier from
/// `cache` (if given). Errors are compile errors; a cache that can't be read
/// or written just means compiling.
pub fn load(engine: &Engine, bytes: &[u8], cache: Option<&Path>) -> wasmtime::Result<Component> {
    let Some(dir) = cache else {
        return Component::new(engine, bytes);
    };
    let path = entry(engine, bytes, dir);
    if path.exists() {
        // SAFETY: `deserialize_file` trusts the file to be machine code that
        // wasmtime itself produced for this engine. It checks the version and
        // settings and refuses mismatches, but it can't detect a deliberately
        // forged file. The cache is in the user's own folder, written only by
        // `serialize` below. Anyone able to put a forged file there can already
        // run programs as this user, so this adds no new way in.
        match unsafe { Component::deserialize_file(engine, &path) } {
            Ok(component) => return Ok(component),
            // Stale or damaged: compile below, replacing it.
            Err(_) => {}
        }
    }
    let component = Component::new(engine, bytes)?;
    if let Ok(serialized) = component.serialize() {
        let _ = write_atomically(&path, &serialized);
    }
    Ok(component)
}

/// The cache file for these bytes on this engine.
fn entry(engine: &Engine, bytes: &[u8], dir: &Path) -> PathBuf {
    let mut hasher = Fnv1a::default();
    engine.precompile_compatibility_hash().hash(&mut hasher);
    bytes.hash(&mut hasher);
    dir.join(format!("{:016x}.cwasm", hasher.finish()))
}

fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    // A name of our own, so two Arrows writing the same entry don't collide.
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

/// FNV-1a, 64 bit. The standard library's hasher is randomly seeded per
/// process, which would make every launch miss; this one gives the same key
/// every time.
struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv1a {
    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest valid component: no imports, no exports.
    const EMPTY_COMPONENT: &[u8] = b"\0asm\x0d\0\x01\0";

    #[test]
    fn a_compiled_plugin_is_saved_and_loaded_back() {
        let dir = Path::new("target/arrow-test-writes/compile-cache");
        let _ = fs::remove_dir_all(dir);
        let engine = Engine::default();

        load(&engine, EMPTY_COMPONENT, Some(dir)).unwrap();
        let entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(entries.len(), 1, "one entry, no temporary files: {entries:?}");
        assert_eq!(entries[0].extension().unwrap(), "cwasm");

        // A second load uses the entry; a damaged one is compiled again.
        load(&engine, EMPTY_COMPONENT, Some(dir)).unwrap();
        fs::write(&entries[0], b"not machine code").unwrap();
        load(&engine, EMPTY_COMPONENT, Some(dir)).unwrap();
        assert!(fs::read(&entries[0]).unwrap().len() > 16, "the damaged entry was replaced");
    }

    #[test]
    fn keys_depend_on_the_bytes_and_stay_the_same() {
        let engine = Engine::default();
        let dir = Path::new("cache");
        assert_eq!(entry(&engine, b"a", dir), entry(&engine, b"a", dir));
        assert_ne!(entry(&engine, b"a", dir), entry(&engine, b"b", dir));
    }
}
