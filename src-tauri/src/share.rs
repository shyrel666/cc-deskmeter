//! Saves an exported share card, when the user clicks "Save PNG": the only
//! file CC-Deskmeter writes outside its config folder, which holds its own
//! `tray.json` and history archive (`archive-*.json`, plus any unreadable
//! one set aside as `*.corrupt.json`).

use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::Local;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// A 2400×1350 card is ~1 MB; anything far larger is not ours.
const MAX_BYTES: usize = 32 * 1024 * 1024;

/// Writes `bytes` (a PNG) into the Downloads folder (falling back to Pictures,
/// then home) under a timestamped name. Never overwrites an existing file.
pub fn save_png(bytes: &[u8]) -> io::Result<PathBuf> {
    if !bytes.starts_with(PNG_SIGNATURE) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a PNG image",
        ));
    }
    if bytes.len() > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "image is too large",
        ));
    }
    let dir = dirs::download_dir()
        .or_else(dirs::picture_dir)
        .or_else(dirs::home_dir)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no Downloads folder"))?;
    write_new(
        &dir,
        &format!("cc-deskmeter-{}", Local::now().format("%Y%m%d-%H%M%S")),
        bytes,
    )
}

fn write_new(dir: &Path, stem: &str, bytes: &[u8]) -> io::Result<PathBuf> {
    for n in 1..1000 {
        let path = match n {
            1 => dir.join(format!("{stem}.png")),
            _ => dir.join(format!("{stem}-{n}.png")),
        };
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(bytes)?;
                return Ok(path);
            }
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "too many cards saved this second",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut bytes = PNG_SIGNATURE.to_vec();
        bytes.extend_from_slice(b"rest");
        bytes
    }

    #[test]
    fn rejects_non_png() {
        let err = save_png(b"<html>").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn never_overwrites() {
        let dir = std::env::temp_dir().join(format!("cc-deskmeter-share-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = write_new(&dir, "card", &png()).unwrap();
        let second = write_new(&dir, "card", &png()).unwrap();
        assert_ne!(first, second);
        assert!(second.ends_with("card-2.png"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
