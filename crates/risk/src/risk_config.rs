//! Value-at-Risk configuration, loaded from `config/risk.toml`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::var::VarMethod;

/// Value-at-Risk settings, loaded and validated from `config/risk.toml`.
///
/// This is deliberately its own file, distinct from `config/limits.toml`: it
/// configures *how* `VaR` is computed and reported, not a ceiling to alert on.
/// [`crate::LimitsConfig::var_99`] (parsed since Part 10) remains the actual
/// limit, evaluated against whichever method [`Self::headline_var_method`]
/// names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RiskConfig {
    /// Which of the three `VaR` methods' `VaR99` the dashboard shows as its
    /// headline figure, and [`crate::LimitsConfig::var_99`] is evaluated
    /// against.
    pub headline_var_method: VarMethod,
}

/// The risk configuration could not be used.
#[derive(Debug, Error)]
pub enum RiskConfigError {
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
    #[error("invalid risk config: {0}")]
    Parse(#[from] toml::de::Error),
}

impl RiskConfig {
    /// Reads a TOML file.
    ///
    /// No further validation is needed beyond parsing: `headline_var_method`
    /// is a closed enum, so a bad value is already a parse error.
    ///
    /// # Errors
    ///
    /// [`RiskConfigError::Read`] or [`RiskConfigError::Parse`].
    pub fn load(path: &Path) -> Result<Self, RiskConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| RiskConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// Parses TOML text.
    ///
    /// # Errors
    ///
    /// [`RiskConfigError::Parse`].
    pub fn from_toml_str(text: &str) -> Result<Self, RiskConfigError> {
        Ok(toml::from_str(text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_the_default_config() {
        let config = RiskConfig::from_toml_str(r#"headline_var_method = "delta_gamma_mc""#)
            .expect("should parse");
        assert_eq!(config.headline_var_method, VarMethod::DeltaGammaMc);
    }

    #[test]
    fn loads_each_valid_method() {
        for (text, expected) in [
            (
                r#"headline_var_method = "delta_normal""#,
                VarMethod::DeltaNormal,
            ),
            (
                r#"headline_var_method = "delta_gamma_mc""#,
                VarMethod::DeltaGammaMc,
            ),
            (
                r#"headline_var_method = "historical""#,
                VarMethod::Historical,
            ),
        ] {
            let config = RiskConfig::from_toml_str(text).expect("should parse");
            assert_eq!(config.headline_var_method, expected);
        }
    }

    #[test]
    fn unknown_method_is_rejected() {
        let err = RiskConfig::from_toml_str(r#"headline_var_method = "bogus""#).unwrap_err();
        assert!(matches!(err, RiskConfigError::Parse(_)));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let err = RiskConfig::from_toml_str(
            r#"headline_var_method = "delta_gamma_mc"
               bogus = 1"#,
        )
        .unwrap_err();
        assert!(matches!(err, RiskConfigError::Parse(_)));
    }

    #[test]
    fn missing_field_is_rejected() {
        let err = RiskConfig::from_toml_str("").unwrap_err();
        assert!(matches!(err, RiskConfigError::Parse(_)));
    }
}
