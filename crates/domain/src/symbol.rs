//! The ticker symbol newtype.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::DomainError;

/// Longest accepted symbol.
const MAX_LEN: usize = 10;

/// A ticker symbol such as `AAPL` or `BRK.B`.
///
/// The constructor trims surrounding whitespace, upper-cases the input and
/// checks it is 1 to 10 characters of A to Z, digits, `.` or `-`, so a value
/// that exists is valid. Deserialising goes through the same check.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Symbol(String);

impl Symbol {
    /// Validates and builds a symbol, upper-casing the input.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidSymbol`] when the trimmed input is empty, longer
    /// than 10 characters or holds a character outside A to Z, 0 to 9, `.` and `-`.
    ///
    /// ```
    /// use domain::Symbol;
    ///
    /// assert_eq!(Symbol::new(" aapl ")?.as_str(), "AAPL");
    /// assert!(Symbol::new("BAD SYMBOL").is_err());
    /// # Ok::<(), domain::DomainError>(())
    /// ```
    pub fn new(input: &str) -> Result<Self, DomainError> {
        let upper = input.trim().to_ascii_uppercase();
        let valid = !upper.is_empty()
            && upper.len() <= MAX_LEN
            && upper
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'.' || b == b'-');
        if valid {
            Ok(Self(upper))
        } else {
            Err(DomainError::InvalidSymbol(input.to_owned()))
        }
    }

    /// The symbol as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Symbol {
    type Err = DomainError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for Symbol {
    type Error = DomainError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<Symbol> for String {
    fn from(symbol: Symbol) -> Self {
        symbol.0
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_cases_and_trims() {
        assert_eq!(
            Symbol::new(" spy ").map(|s| s.to_string()),
            Ok("SPY".into())
        );
    }

    #[test]
    fn accepts_dots_dashes_and_digits() {
        assert!(Symbol::new("BRK.B").is_ok());
        assert!(Symbol::new("RDS-A").is_ok());
        assert!(Symbol::new("3M").is_ok());
    }

    #[test]
    fn length_limits() {
        assert!(Symbol::new("A").is_ok());
        assert!(Symbol::new("ABCDEFGHIJ").is_ok());
        assert!(Symbol::new("ABCDEFGHIJK").is_err());
        assert!(Symbol::new("").is_err());
        assert!(Symbol::new("   ").is_err());
    }

    #[test]
    fn rejects_bad_characters() {
        for bad in ["AA PL", "AAPL!", "A_B", "É", "A/B", "AA\tPL"] {
            assert!(Symbol::new(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn from_str_and_display_round_trip() {
        let s: Symbol = "msft".parse().unwrap();
        assert_eq!(s.to_string(), "MSFT");
        assert_eq!(s.as_ref(), "MSFT");
    }

    #[test]
    fn ordering_is_alphabetical() {
        let a = Symbol::new("AAPL").unwrap();
        let b = Symbol::new("MSFT").unwrap();
        assert!(a < b);
    }

    #[test]
    fn serde_validates_on_the_way_in() {
        let s: Symbol = serde_json::from_str("\"nvda\"").unwrap();
        assert_eq!(s.as_str(), "NVDA");
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"NVDA\"");
        assert!(serde_json::from_str::<Symbol>("\"not valid\"").is_err());
    }
}
