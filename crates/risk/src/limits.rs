//! Risk limits configuration, loaded from `config/limits.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use domain::Symbol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Portfolio-wide limits.
///
/// Every field but `max_concentration` is a dollar or vol-point ceiling on an absolute
/// value; `theta_floor` is the exception, described on its own doc comment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioLimits {
    /// Max |portfolio dollar delta|. Directional exposure.
    pub dollar_delta: f64,
    /// Max |portfolio dollar gamma 1 %|. How fast delta changes, the hedging cost in a
    /// fast market.
    pub dollar_gamma_1pct: f64,
    /// Max |portfolio vega| per vol point. Exposure to implied vol moves.
    pub vega: f64,
    /// Max daily time decay the book pays. Compared against `max(0, -theta)`, so a
    /// portfolio with flat or positive theta never breaches this limit.
    pub theta_floor: f64,
    /// Max gross exposure, Σ|dollar delta| over underlyings.
    pub gross_exposure: f64,
    /// Max share of gross exposure any single underlying may hold, in (0, 1]. Concentration.
    pub max_concentration: f64,
}

/// Per-underlying |dollar delta| limit: a default plus optional per-symbol overrides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnderlyingDeltaLimit {
    /// The limit for a symbol with no override.
    pub default: f64,
    /// Per-symbol limits that replace the default.
    #[serde(default)]
    pub overrides: BTreeMap<Symbol, f64>,
}

impl UnderlyingDeltaLimit {
    /// The limit for `symbol`: its override if one exists, else the default.
    pub fn limit_for(&self, symbol: &Symbol) -> f64 {
        self.overrides.get(symbol).copied().unwrap_or(self.default)
    }
}

/// The limits configuration could not be used.
#[derive(Debug, Error)]
pub enum LimitsConfigError {
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
    #[error("invalid limits config: {0}")]
    Parse(#[from] toml::de::Error),
    /// A value broke a rule.
    #[error("invalid limits config: {0}")]
    Invalid(String),
}

/// Risk limits, loaded and validated from `config/limits.toml`.
///
/// Load with [`LimitsConfig::load`] or [`LimitsConfig::from_toml_str`], both of which
/// validate: every limit is positive and finite, `max_concentration` is in (0, 1], and
/// `warning_clear_threshold < warning_threshold < clear_threshold < 1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsConfig {
    /// Utilisation at or above this fraction of a limit is a warning.
    pub warning_threshold: f64,
    /// Once a limit reaches `Warning` (or higher), it stays at `Warning` or above until
    /// utilisation drops below this fraction. Sitting strictly below `warning_threshold`,
    /// this stops a value that hovers around the warning line from flapping between
    /// `Warning` and `Ok` on every tick.
    pub warning_clear_threshold: f64,
    /// Once breached, a limit clears only when utilisation drops below this fraction.
    /// Sitting strictly between `warning_threshold` and 1, this stops a value that
    /// hovers around the breach line from flapping between `Breach` and a lower state
    /// on every tick.
    pub clear_threshold: f64,
    /// Portfolio-wide limits.
    pub portfolio: PortfolioLimits,
    /// Per-underlying dollar delta limit.
    pub underlying_dollar_delta: UnderlyingDeltaLimit,
    /// 99 % one-day Value at Risk limit. Parsed but not evaluated until Part 18.
    pub var_99: Option<f64>,
}

impl LimitsConfig {
    /// Reads and validates a TOML file.
    ///
    /// # Errors
    ///
    /// [`LimitsConfigError::Read`], [`LimitsConfigError::Parse`] or
    /// [`LimitsConfigError::Invalid`].
    pub fn load(path: &Path) -> Result<Self, LimitsConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| LimitsConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_toml_str(&text)
    }

    /// Parses and validates TOML text.
    ///
    /// # Errors
    ///
    /// [`LimitsConfigError::Parse`] or [`LimitsConfigError::Invalid`].
    pub fn from_toml_str(text: &str) -> Result<Self, LimitsConfigError> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Checks the rules the type cannot enforce.
    ///
    /// # Errors
    ///
    /// [`LimitsConfigError::Invalid`] naming the first broken rule.
    pub fn validate(&self) -> Result<(), LimitsConfigError> {
        positive("portfolio.dollar_delta", self.portfolio.dollar_delta)?;
        positive(
            "portfolio.dollar_gamma_1pct",
            self.portfolio.dollar_gamma_1pct,
        )?;
        positive("portfolio.vega", self.portfolio.vega)?;
        positive("portfolio.theta_floor", self.portfolio.theta_floor)?;
        positive("portfolio.gross_exposure", self.portfolio.gross_exposure)?;
        if !(self.portfolio.max_concentration.is_finite()
            && 0.0 < self.portfolio.max_concentration
            && self.portfolio.max_concentration <= 1.0)
        {
            return Err(LimitsConfigError::Invalid(format!(
                "portfolio.max_concentration must be in (0, 1], got {}",
                self.portfolio.max_concentration
            )));
        }
        positive(
            "underlying_dollar_delta.default",
            self.underlying_dollar_delta.default,
        )?;
        for (symbol, value) in &self.underlying_dollar_delta.overrides {
            positive(
                &format!("underlying_dollar_delta.overrides.{symbol}"),
                *value,
            )?;
        }
        if let Some(var_99) = self.var_99 {
            positive("var_99", var_99)?;
        }
        if !self.warning_clear_threshold.is_finite()
            || !self.warning_threshold.is_finite()
            || !self.clear_threshold.is_finite()
        {
            return Err(LimitsConfigError::Invalid(
                "warning_clear_threshold, warning_threshold and clear_threshold must be finite"
                    .into(),
            ));
        }
        if !(0.0 < self.warning_clear_threshold
            && self.warning_clear_threshold < self.warning_threshold
            && self.warning_threshold < self.clear_threshold
            && self.clear_threshold < 1.0)
        {
            return Err(LimitsConfigError::Invalid(format!(
                "thresholds must satisfy 0 < warning_clear_threshold ({}) < warning_threshold ({}) < clear_threshold ({}) < 1",
                self.warning_clear_threshold, self.warning_threshold, self.clear_threshold
            )));
        }
        Ok(())
    }
}

/// Errors if `value` is not finite and strictly positive, naming `field` in the message.
fn positive(field: &str, value: f64) -> Result<(), LimitsConfigError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(LimitsConfigError::Invalid(format!(
            "{field} must be positive, got {value}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    fn valid_toml() -> &'static str {
        r"
            warning_clear_threshold = 0.75
            warning_threshold = 0.8
            clear_threshold = 0.95
            var_99 = 50000

            [portfolio]
            dollar_delta = 250000
            dollar_gamma_1pct = 15000
            vega = 8000
            theta_floor = 4000
            gross_exposure = 600000
            max_concentration = 0.5

            [underlying_dollar_delta]
            default = 100000
            [underlying_dollar_delta.overrides]
            NVDA = 60000
        "
    }

    #[test]
    fn loads_a_valid_config() {
        let config = LimitsConfig::from_toml_str(valid_toml()).unwrap();
        assert_relative_eq!(config.portfolio.dollar_delta, 250_000.0);
        assert_relative_eq!(
            config
                .underlying_dollar_delta
                .limit_for(&Symbol::new("NVDA").unwrap()),
            60_000.0
        );
        assert_relative_eq!(
            config
                .underlying_dollar_delta
                .limit_for(&Symbol::new("AAPL").unwrap()),
            100_000.0
        );
        assert_relative_eq!(config.var_99.unwrap(), 50_000.0);
    }

    #[test]
    fn negative_limit_is_rejected() {
        let text = valid_toml().replace("dollar_delta = 250000", "dollar_delta = -250000");
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Invalid(_)));
    }

    #[test]
    fn zero_limit_is_rejected() {
        let text = valid_toml().replace("vega = 8000", "vega = 0");
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Invalid(_)));
    }

    #[test]
    fn bad_thresholds_are_rejected() {
        let text = valid_toml().replace("clear_threshold = 0.95", "clear_threshold = 0.7");
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Invalid(_)));
    }

    #[test]
    fn warning_threshold_at_or_above_one_is_rejected() {
        let text = valid_toml().replace("warning_threshold = 0.8", "warning_threshold = 1.0");
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Invalid(_)));
    }

    #[test]
    fn warning_clear_threshold_at_or_above_warning_threshold_is_rejected() {
        let text = valid_toml().replace(
            "warning_clear_threshold = 0.75",
            "warning_clear_threshold = 0.8",
        );
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Invalid(_)));
    }

    #[test]
    fn concentration_out_of_range_is_rejected() {
        let text = valid_toml().replace("max_concentration = 0.5", "max_concentration = 1.5");
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Invalid(_)));
    }

    #[test]
    fn unknown_field_is_rejected() {
        let text = valid_toml().replace("var_99 = 50000", "var_99 = 50000\nbogus = 1");
        let err = LimitsConfig::from_toml_str(&text).unwrap_err();
        assert!(matches!(err, LimitsConfigError::Parse(_)));
    }
}
