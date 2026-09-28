//! The storage configuration, loaded from `config/storage.toml`. Mirrors
//! `marketdata::MarketConfig`'s own load/validate pattern.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// Which book a new session starts from.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(try_from = "String")]
pub enum StartPortfolio {
    /// `portfolios/current.json` if it exists, else the sample CSV.
    Last,
    /// Always the sample CSV at `ServerConfig::portfolio_path`.
    Sample,
    /// A specific file under `portfolios/versions/`.
    Version(String),
}

impl TryFrom<String> for StartPortfolio {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        match value.as_str() {
            "last" => Ok(Self::Last),
            "sample" => Ok(Self::Sample),
            other => {
                engine::check_part(other).map_err(|err| err.to_string())?;
                Ok(Self::Version(other.to_owned()))
            }
        }
    }
}

/// The storage configuration could not be used.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// The TOML did not parse into the expected shape.
    #[error("invalid storage config: {0}")]
    Parse(#[from] toml::de::Error),
    /// A value broke a rule.
    #[error("invalid storage config: {0}")]
    Invalid(String),
}

/// Local storage settings: which book to start from, and the soft size ceiling the
/// background usage monitor warns against. Nothing is ever deleted automatically,
/// regardless of these values.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Which book a new session starts from.
    pub start_portfolio: StartPortfolio,
    /// Soft ceiling on the storage folder's total size, in bytes.
    pub max_bytes: u64,
    /// Fraction of `max_bytes` at or above which the size monitor warns.
    pub warn_fraction: f64,
}

impl StorageConfig {
    /// Reads and validates a TOML file.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Read`], [`ConfigError::Parse`] or [`ConfigError::Invalid`].
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// Parses and validates TOML text.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Parse`] or [`ConfigError::Invalid`].
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Checks the rules the type cannot enforce: `max_bytes` positive, `warn_fraction`
    /// in `(0, 1]`.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] naming the first broken rule.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_bytes == 0 {
            return Err(ConfigError::Invalid("max_bytes must be positive".into()));
        }
        if !(self.warn_fraction.is_finite()
            && self.warn_fraction > 0.0
            && self.warn_fraction <= 1.0)
        {
            return Err(ConfigError::Invalid(format!(
                "warn_fraction must be in (0, 1], got {}",
                self.warn_fraction
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
        start_portfolio = "last"
        max_bytes = 21474836480
        warn_fraction = 0.8
    "#;

    #[test]
    fn parses_a_valid_config() {
        let c = StorageConfig::from_toml_str(GOOD).unwrap();
        assert_eq!(c.start_portfolio, StartPortfolio::Last);
        assert_eq!(c.max_bytes, 21_474_836_480);
    }

    #[test]
    fn start_portfolio_forms() {
        assert_eq!(
            StorageConfig::from_toml_str(&GOOD.replace("\"last\"", "\"sample\""))
                .unwrap()
                .start_portfolio,
            StartPortfolio::Sample
        );
        assert_eq!(
            StorageConfig::from_toml_str(
                &GOOD.replace("\"last\"", "\"20260926T041500Z-mybook.csv\"")
            )
            .unwrap()
            .start_portfolio,
            StartPortfolio::Version("20260926T041500Z-mybook.csv".to_owned())
        );
    }

    #[test]
    fn an_unsafe_version_name_is_rejected() {
        for bad in ["..", "../etc/passwd", "a/b"] {
            let text = GOOD.replace("\"last\"", &format!("{bad:?}"));
            assert!(StorageConfig::from_toml_str(&text).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rejects_broken_configs() {
        let cases = [
            GOOD.replace("21474836480", "0"),
            GOOD.replace("0.8", "0.0"),
            GOOD.replace("0.8", "1.5"),
            GOOD.replace("0.8", "nan"),
            format!("{GOOD}\nextra = 1"),
        ];
        for (i, text) in cases.iter().enumerate() {
            assert!(StorageConfig::from_toml_str(text).is_err(), "case {i}");
        }
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let err = StorageConfig::load(Path::new("/definitely/not/here.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
