//! Recording persistence.
//!
//! JSON was chosen over a binary format because recordings are moderate in
//! size, JSON is trivially inspectable and recoverable, and the schema can be
//! versioned without adopting a fragile custom parser. Writes are atomic:
//! content is written and fsynced to a temporary file in the destination
//! directory, then renamed over the target.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::error::StorageError;
use crate::model::{Recording, FILE_EXTENSION, MAX_FILE_BYTES};

/// Add the canonical extension when the user did not type one.
pub fn ensure_extension(path: PathBuf) -> PathBuf {
    match path.extension() {
        Some(ext) if !ext.is_empty() => path,
        _ => path.with_extension(FILE_EXTENSION),
    }
}

pub fn load_recording(path: &Path) -> Result<Recording, StorageError> {
    let metadata = fs::metadata(path).map_err(|error| StorageError::io(path, error))?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(StorageError::TooLarge {
            size: metadata.len(),
        });
    }

    let bytes = fs::read(path).map_err(|error| StorageError::io(path, error))?;
    let recording: Recording = serde_json::from_slice(&bytes).map_err(StorageError::Malformed)?;
    recording.validate()?;
    Ok(recording)
}

pub fn save_recording(path: &Path, recording: &Recording) -> Result<(), StorageError> {
    recording.validate()?;
    write_json_atomic(path, recording)
}

/// Generic atomic JSON write used for recordings, settings, and recovery data.
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(StorageError::Malformed)?;
    write_bytes_atomic(path, &bytes).map_err(|error| StorageError::io(path, error))
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording");
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!(".{file_name}.tmp-{}-{stamp}", std::process::id()));

    let result = (|| -> io::Result<()> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(bytes)?;
        writer.write_all(b"\n")?;
        writer.flush()?;
        writer.get_ref().sync_all()?;
        drop(writer);
        fs::rename(&temp_path, path)?;

        // fsync the directory too, so the rename survives a power loss on
        // filesystems that require it.
        if let Ok(directory) = File::open(parent) {
            let _ = directory.sync_all();
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

#[cfg(test)]
mod tests;
