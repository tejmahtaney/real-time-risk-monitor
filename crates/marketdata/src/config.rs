//! The market configuration, loaded from `config/market.toml`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use domain::Symbol;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Where the loader looks for history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataMode {
    /// Local archive, then the API, then the fallback files.
    Auto,
    /// Never call the API: local archive, then the fallback files.
    Offline,
}

/// Which volatility estimate seeds the simulator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VolMethod {
    /// Sample standard deviation of the last `vol_window_days` log returns.
    Realised,
    /// `RiskMetrics` exponentially weighted moving average with lambda 0.94.
    Ewma,
}

/// How a symbol's dividends enter option pricing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DividendModel {
    /// The continuous yield q of the `dividend_yields` table.
    #[default]
    Yield,
    /// Discrete cash dividends from the corporate-action calendar, priced with the
    /// escrowed model. The symbol's continuous yield is ignored and treated as zero.
    Discrete,
}

/// The configuration file could not be used.
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
    #[error("invalid market config: {0}")]
    Parse(#[from] toml::de::Error),
    /// A value broke a rule.
    #[error("invalid market config: {0}")]
    Invalid(String),
}

/// Market inputs for the real-data seed.
///
/// Rates, yields and vols are decimals (0.05 means 5 %). The rate is continuously
/// compounded. Load with [`MarketConfig::load`] or [`MarketConfig::from_toml_str`], both
/// of which validate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketConfig {
    /// The underlyings to load, in the order given.
    pub symbols: Vec<Symbol>,
    /// Continuously compounded risk-free rate.
    pub risk_free_rate: f64,
    /// Continuous dividend yield per symbol. Required for every symbol, but ignored (as
    /// zero) for a symbol on the discrete model.
    pub dividend_yields: BTreeMap<Symbol, f64>,
    /// The dividend model per symbol. A symbol not listed uses [`DividendModel::Yield`].
    #[serde(default)]
    pub dividend_model: BTreeMap<Symbol, DividendModel>,
    /// Number of daily returns in the vol and correlation window.
    pub vol_window_days: usize,
    /// Which vol estimate seeds the simulator.
    pub vol_method: VolMethod,
    /// An archived download younger than this many hours is a cache hit.
    pub cache_max_age_hours: u64,
    /// Where history may come from.
    pub data_mode: DataMode,
}

impl MarketConfig {
    /// The dividend model of `symbol`: [`DividendModel::Yield`] unless the config says otherwise.
    pub fn model_of(&self, symbol: &Symbol) -> DividendModel {
        self.dividend_model.get(symbol).copied().unwrap_or_default()
    }

    /// The symbols on the discrete dividend model, in config order.
    pub fn discrete_symbols(&self) -> Vec<Symbol> {
        self.symbols
            .iter()
            .filter(|s| self.model_of(s) == DividendModel::Discrete)
            .cloned()
            .collect()
    }

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

    /// Checks the rules the type cannot enforce: at least one unique symbol, a window of
    /// at least 2 returns, a finite rate, and a finite yield in [0, 1) for every symbol.
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] naming the first broken rule.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |msg: String| Err(ConfigError::Invalid(msg));
        if self.symbols.is_empty() {
            return invalid("symbols must not be empty".into());
        }
        for (i, symbol) in self.symbols.iter().enumerate() {
            if self.symbols.iter().take(i).any(|s| s == symbol) {
                return invalid(format!("symbol {symbol} is listed twice"));
            }
            if !self.dividend_yields.contains_key(symbol) {
                return invalid(format!("no dividend yield for {symbol}"));
            }
        }
        if let Some(extra) = self
            .dividend_yields
            .keys()
            .find(|s| !self.symbols.contains(s))
        {
            return invalid(format!("dividend yield given for unlisted symbol {extra}"));
        }
        if let Some(extra) = self
            .dividend_model
            .keys()
            .find(|s| !self.symbols.contains(s))
        {
            return invalid(format!("dividend model given for unlisted symbol {extra}"));
        }
        if !self.risk_free_rate.is_finite() {
            return invalid(format!(
                "risk_free_rate must be finite, got {}",
                self.risk_free_rate
            ));
        }
        for (symbol, q) in &self.dividend_yields {
            if !q.is_finite() || !(0.0..1.0).contains(q) {
                return invalid(format!(
                    "dividend yield for {symbol} must be in [0, 1), got {q}"
                ));
            }
        }
        if self.vol_window_days < 2 {
            return invalid(format!(
                "vol_window_days must be at least 2, got {}",
                self.vol_window_days
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    const GOOD: &str = r#"
        symbols = ["SPY", "TSLA"]
        risk_free_rate = 0.04
        vol_window_days = 60
        vol_method = "ewma"
        cache_max_age_hours = 24
        data_mode = "offline"
        [dividend_yields]
        SPY = 0.01
        TSLA = 0.0
    "#;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    #[test]
    fn parses_a_valid_config() {
        let c = MarketConfig::from_toml_str(GOOD).unwrap();
        assert_eq!(c.symbols, [sym("SPY"), sym("TSLA")]);
        assert_eq!(c.vol_method, VolMethod::Ewma);
        assert_eq!(c.data_mode, DataMode::Offline);
        assert_relative_eq!(c.dividend_yields[&sym("SPY")], 0.01);
        assert_eq!(c.cache_max_age_hours, 24);
    }

    #[test]
    fn dividend_models_default_to_the_yield() {
        let c = MarketConfig::from_toml_str(GOOD).unwrap();
        assert_eq!(c.model_of(&sym("SPY")), DividendModel::Yield);
        assert!(c.discrete_symbols().is_empty());
        let with_model = format!("{GOOD}\n[dividend_model]\nTSLA = \"discrete\"");
        let c = MarketConfig::from_toml_str(&with_model).unwrap();
        assert_eq!(c.discrete_symbols(), [sym("TSLA")]);
        assert_eq!(c.model_of(&sym("SPY")), DividendModel::Yield);
    }

    #[test]
    fn rejects_broken_configs() {
        let cases = [
            GOOD.replace(r#"["SPY", "TSLA"]"#, "[]"),
            GOOD.replace(r#"["SPY", "TSLA"]"#, r#"["SPY", "SPY"]"#),
            GOOD.replace("TSLA = 0.0", ""),
            GOOD.replace("SPY = 0.01", "SPY = 0.01\nAAPL = 0.01"),
            format!("{GOOD}\n[dividend_model]\nAAPL = \"discrete\""),
            format!("{GOOD}\n[dividend_model]\nSPY = \"continuous\""),
            GOOD.replace("SPY = 0.01", "SPY = 1.5"),
            GOOD.replace("SPY = 0.01", "SPY = -0.01"),
            GOOD.replace("vol_window_days = 60", "vol_window_days = 1"),
            GOOD.replace("risk_free_rate = 0.04", "risk_free_rate = nan"),
            GOOD.replace("\"ewma\"", "\"garch\""),
            GOOD.replace("\"offline\"", "\"online\""),
            GOOD.replace("SPY\", \"TSLA", "sp y\", \"TSLA"),
            format!("{GOOD}\nextra = 1"),
        ];
        for (i, text) in cases.iter().enumerate() {
            assert!(MarketConfig::from_toml_str(text).is_err(), "case {i}");
        }
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let err = MarketConfig::load(Path::new("/definitely/not/here.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }
}
