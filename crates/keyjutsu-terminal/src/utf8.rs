//! UTF-8 decoding across read boundaries.
//!
//! A pseudo-console read can end half-way through a multi-byte character.
//! Decoding each read on its own would turn that character into two
//! replacement characters on screen, so the incomplete tail is carried into
//! the next read instead.

#[derive(Debug, Default)]
pub struct Utf8Carry {
    tail: Vec<u8>,
}

impl Utf8Carry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decode as much of `data` as forms complete characters. Bytes that can
    /// never be valid are replaced with U+FFFD, as a terminal would show them.
    pub fn decode(&mut self, data: &[u8]) -> String {
        let mut buf = std::mem::take(&mut self.tail);
        buf.extend_from_slice(data);
        let mut out = String::with_capacity(buf.len());
        let mut rest = buf.as_slice();
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    break;
                }
                Err(e) => {
                    let (valid, after) = rest.split_at(e.valid_up_to());
                    // `valid_up_to` guarantees this prefix is UTF-8.
                    out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match e.error_len() {
                        Some(bad) => {
                            out.push('\u{FFFD}');
                            rest = &after[bad..];
                        }
                        None => {
                            self.tail = after.to_vec();
                            break;
                        }
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_split_across_reads_is_decoded_once_complete() {
        let text = "déjà ✓ 日本";
        let bytes = text.as_bytes();
        for split in 0..=bytes.len() {
            let mut carry = Utf8Carry::new();
            let mut out = carry.decode(&bytes[..split]);
            out.push_str(&carry.decode(&bytes[split..]));
            assert_eq!(out, text, "split at {split}");
        }
    }

    #[test]
    fn invalid_bytes_become_replacement_characters() {
        let mut carry = Utf8Carry::new();
        assert_eq!(carry.decode(b"a\xffb"), "a\u{FFFD}b");
    }
}
