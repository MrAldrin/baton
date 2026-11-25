//! Key names for `baton keys`.

use anyhow::{Result, bail};

/// Bytes for a key name: `enter`, `esc`, `tab`, `up`, `down`, `ctrl-c`,
/// or one digit.
///
/// ```
/// assert_eq!(baton::keys::key_bytes("enter").unwrap(), b"\r");
/// assert_eq!(baton::keys::key_bytes("2").unwrap(), b"2");
/// ```
///
/// # Errors
/// The name is not a known key.
pub fn key_bytes(name: &str) -> Result<&'static [u8]> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "enter" => b"\r",
        "esc" => b"\x1b",
        "tab" => b"\t",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        "ctrl-c" => b"\x03",
        "0" => b"0",
        "1" => b"1",
        "2" => b"2",
        "3" => b"3",
        "4" => b"4",
        "5" => b"5",
        "6" => b"6",
        "7" => b"7",
        "8" => b"8",
        "9" => b"9",
        _ => bail!("unknown key {name:?}: use enter, esc, tab, up, down, ctrl-c or a digit"),
    })
}

#[cfg(test)]
mod tests {
    use super::key_bytes;

    #[test]
    fn known_keys() {
        assert_eq!(key_bytes("ESC").unwrap(), b"\x1b");
        assert_eq!(key_bytes("up").unwrap(), b"\x1b[A");
        assert_eq!(key_bytes("ctrl-c").unwrap(), b"\x03");
    }

    #[test]
    fn unknown_key() {
        assert!(key_bytes("y").is_err());
        assert!(key_bytes("12").is_err());
    }
}
