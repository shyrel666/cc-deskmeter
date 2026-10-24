//! Streaming decompression of `.jsonl.zst` rollouts: a rollout is read line
//! by line as it is decoded, so memory stays at the decoder's window however
//! large the rollout (the largest seen holds 46 MB of JSON). Plain rollouts
//! are read incrementally by the store.

use std::io::{self, BufRead, Read};

use ruzstd::decoding::errors::{FrameDecoderError, ReadFrameHeaderError};
use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};

/// Every frame of a Zstandard stream in turn, skippable frames skipped. A
/// frame that carries a checksum is verified once drained, so a truncated or
/// damaged file (one Codex is still writing, say) fails instead of yielding
/// part of a rollout.
pub struct ZstReader<R> {
    source: R,
    decoder: FrameDecoder,
    in_frame: bool,
}

impl<R: BufRead> ZstReader<R> {
    pub fn new(source: R) -> Self {
        Self {
            source,
            decoder: FrameDecoder::new(),
            in_frame: false,
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

impl<R: BufRead> Read for ZstReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if !self.in_frame {
                if self.source.fill_buf()?.is_empty() {
                    return Ok(0);
                }
                match self.decoder.reset(&mut self.source) {
                    Ok(()) => self.in_frame = true,
                    Err(FrameDecoderError::ReadFrameHeaderError(
                        ReadFrameHeaderError::SkipFrame { length, .. },
                    )) => {
                        let length = u64::from(length);
                        let skipped =
                            io::copy(&mut (&mut self.source).take(length), &mut io::sink())?;
                        if skipped < length {
                            return Err(invalid("truncated skippable frame"));
                        }
                    }
                    Err(err) => return Err(invalid(format!("not a Zstandard rollout: {err}"))),
                }
                continue;
            }
            // Decode until something can be handed out or the frame ends; a
            // frame cut short fails here, reading past the end of the file.
            while self.decoder.can_collect() == 0 && !self.decoder.is_finished() {
                self.decoder
                    .decode_blocks(&mut self.source, BlockDecodingStrategy::UptoBlocks(1))
                    .map_err(|err| invalid(err.to_string()))?;
            }
            let read = self.decoder.read(buf)?;
            if read > 0 {
                return Ok(read);
            }
            if let Some(expected) = self.decoder.get_checksum_from_data()
                && self.decoder.get_calculated_checksum() != Some(expected)
            {
                return Err(invalid("Zstandard checksum mismatch"));
            }
            self.in_frame = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decompress(input: &[u8]) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        ZstReader::new(input).read_to_end(&mut output)?;
        Ok(output)
    }

    /// `a\nb\n` as one frame with a content checksum.
    const FRAME: [u8; 17] = [
        0x28, 0xb5, 0x2f, 0xfd, 0x24, 0x04, 0x21, 0x00, 0x00, 0x61, 0x0a, 0x62, 0x0a, 0x25, 0x50,
        0xb3, 0x23,
    ];

    #[test]
    fn decodes_concatenated_and_skippable_frames() {
        assert_eq!(decompress(&FRAME).unwrap(), b"a\nb\n");
        let mut two = FRAME.to_vec();
        // A skippable frame (magic 0x184D2A50, 3 bytes) between two frames.
        two.extend([0x50, 0x2a, 0x4d, 0x18, 0x03, 0x00, 0x00, 0x00, 1, 2, 3]);
        two.extend(FRAME);
        assert_eq!(decompress(&two).unwrap(), b"a\nb\na\nb\n");
        assert!(decompress(&[]).unwrap().is_empty());
    }

    #[test]
    fn rejects_truncated_corrupt_and_foreign_input() {
        assert!(
            decompress(&FRAME[..FRAME.len() - 2]).is_err(),
            "checksum cut off"
        );
        let mut corrupt = FRAME;
        corrupt[10] ^= 0xff;
        assert!(
            decompress(&corrupt).is_err(),
            "content no longer matches its checksum"
        );
        assert!(decompress(b"{\"type\":\"session_meta\"}\n").is_err());
    }
}
