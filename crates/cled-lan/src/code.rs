//! Pairing codes: 8 Crockford base32 characters (40 bits), shown as `XXXX-XXXX`.

use std::fmt;

use crate::{LanError, Result};

/// Crockford's base32 alphabet: no I, L, O, or U, so codes are hard to misread.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const LEN: usize = 8;

#[derive(Clone, PartialEq, Eq)]
pub struct PairingCode(String);

impl PairingCode {
    pub fn generate() -> Result<Self> {
        let mut bytes = [0u8; 5]; // 40 bits = 8 × 5-bit characters
        getrandom::fill(&mut bytes).map_err(|err| LanError::Other(err.to_string()))?;
        let bits = bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b));
        let code = (0..LEN)
            .rev()
            .map(|i| char::from(ALPHABET[((bits >> (i * 5)) & 0x1f) as usize]))
            .collect();
        Ok(Self(code))
    }

    /// Parses what a person typed: case-insensitive, ignores spaces and dashes, and accepts the
    /// usual misreadings (I/L → 1, O → 0).
    pub fn parse(input: &str) -> Result<Self> {
        let code: String = input
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-')
            .map(|c| match c.to_ascii_uppercase() {
                'I' | 'L' => '1',
                'O' => '0',
                other => other,
            })
            .collect();
        if code.len() != LEN || !code.bytes().all(|b| ALPHABET.contains(&b)) {
            return Err(LanError::WrongCode);
        }
        Ok(Self(code))
    }

    /// The canonical form used as the SPAKE2 password.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Display for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", &self.0[..4], &self.0[4..])
    }
}

impl fmt::Debug for PairingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PairingCode(****-****)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_codes_are_well_formed_and_vary() {
        let a = PairingCode::generate().unwrap();
        let b = PairingCode::generate().unwrap();
        assert_ne!(a, b);
        assert_eq!(a.to_string().len(), 9);
        assert_eq!(PairingCode::parse(&a.to_string()).unwrap(), a);
    }

    #[test]
    fn parse_is_forgiving_about_formatting() {
        let code = PairingCode::parse("k7m2-9qxd").unwrap();
        assert_eq!(code.to_string(), "K7M2-9QXD");
        assert_eq!(PairingCode::parse(" K7M2 9QXD ").unwrap(), code);
        assert_eq!(
            PairingCode::parse("IO000000").unwrap().to_string(),
            "1000-0000"
        );
    }

    #[test]
    fn parse_rejects_bad_codes() {
        assert!(PairingCode::parse("K7M2-9QX").is_err());
        assert!(PairingCode::parse("K7M2-9QXDZ").is_err());
        assert!(PairingCode::parse("K7M2-9QX!").is_err());
        assert!(PairingCode::parse("K7M2-9QXU").is_err()); // U isn't in the alphabet
    }

    #[test]
    fn debug_does_not_reveal_the_code() {
        let code = PairingCode::generate().unwrap();
        assert!(!format!("{code:?}").contains(&code.0));
    }
}
