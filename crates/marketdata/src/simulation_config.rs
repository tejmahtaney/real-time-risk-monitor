//! The simulation configuration, loaded from `config/simulation.toml`.

use std::path::Path;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Utc};
use domain::next_session_open_after;
use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::seed::SeedData;

/// The mean-reverting log-vol process shared by every symbol.
///
/// For each symbol x = ln σ follows an Ornstein-Uhlenbeck process that reverts to
/// ln(seed vol). Rates are per trading year.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VolProcessConfig {
    /// Speed of mean reversion κ, per trading year. Must be positive.
    pub mean_reversion: f64,
    /// Vol of vol ξ: standard deviation of log vol per √(trading year). Zero switches
    /// the vol process off.
    pub vol_of_vol: f64,
    /// Correlation ρ between a symbol's price shock and its vol shock, in [−1, 1].
    /// Negative gives the leverage effect.
    pub spot_vol_correlation: f64,
}

/// Settings for the simulated price feed and its clock.
///
/// Load with [`SimulationConfig::load`] or [`SimulationConfig::from_toml_str`], both of
/// which validate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationConfig {
    /// Seed of the `ChaCha8` random stream.
    pub seed: u64,
    /// Real milliseconds between engine ticks. The engine (Part 11) reads it.
    pub tick_interval_ms: u64,
    /// Simulated seconds per real second.
    pub speed: f64,
    /// Annual drift μ of every underlying, as a decimal.
    pub drift: f64,
    /// The first session, as a New York date. `None` starts at the first session after
    /// the seed data's last close.
    #[serde(default)]
    pub start_date: Option<NaiveDate>,
    /// The log-vol process.
    pub vol_process: VolProcessConfig,
}

impl SimulationConfig {
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

    /// Checks the rules the type cannot enforce: a positive tick interval, a finite
    /// positive speed, a finite drift, a positive mean reversion, a finite non-negative
    /// vol of vol and a spot-vol correlation in [−1, 1].
    ///
    /// # Errors
    ///
    /// [`ConfigError::Invalid`] naming the first broken rule.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let invalid = |msg: String| Err(ConfigError::Invalid(format!("simulation: {msg}")));
        let vol = &self.vol_process;
        if self.tick_interval_ms == 0 {
            return invalid("tick_interval_ms must be positive".into());
        }
        if !self.speed.is_finite() || self.speed <= 0.0 {
            return invalid(format!(
                "speed must be finite and positive, got {}",
                self.speed
            ));
        }
        if !self.drift.is_finite() {
            return invalid(format!("drift must be finite, got {}", self.drift));
        }
        if !vol.mean_reversion.is_finite() || vol.mean_reversion <= 0.0 {
            return invalid(format!(
                "mean_reversion must be finite and positive, got {}",
                vol.mean_reversion
            ));
        }
        if !vol.vol_of_vol.is_finite() || vol.vol_of_vol < 0.0 {
            return invalid(format!(
                "vol_of_vol must be finite and not negative, got {}",
                vol.vol_of_vol
            ));
        }
        if !vol.spot_vol_correlation.is_finite()
            || !(-1.0..=1.0).contains(&vol.spot_vol_correlation)
        {
            return invalid(format!(
                "spot_vol_correlation must be in [-1, 1], got {}",
                vol.spot_vol_correlation
            ));
        }
        Ok(())
    }

    /// Real time between engine ticks.
    pub fn tick_interval(&self) -> Duration {
        Duration::from_millis(self.tick_interval_ms)
    }

    /// Where the simulated clock should start.
    ///
    /// With `start_date` set, the open of that New York date, or of the next weekday if it
    /// is a weekend. Otherwise the first session open after the seed data's last close.
    /// The clock itself snaps a start that falls outside a session, so this returns the
    /// open in both cases.
    pub fn start_time(&self, seed: &SeedData) -> DateTime<Utc> {
        match self.start_date {
            Some(date) => match date.pred_opt() {
                Some(day_before) => next_session_open_after(day_before),
                None => next_session_open_after(date),
            },
            None => next_session_open_after(seed.as_of),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use approx::assert_relative_eq;
    use chrono::TimeZone;

    use super::*;

    const GOOD: &str = r"
        seed = 42
        tick_interval_ms = 250
        speed = 120
        drift = 0.0
        [vol_process]
        mean_reversion = 5.0
        vol_of_vol = 0.9
        spot_vol_correlation = -0.7
    ";

    fn seed_as_of(as_of: NaiveDate) -> SeedData {
        SeedData {
            as_of,
            rate: 0.04,
            symbols: Vec::new(),
            underlyings: BTreeMap::new(),
            return_dates: Vec::new(),
            returns: Vec::new(),
            correlation: Vec::new(),
        }
    }

    #[test]
    fn parses_the_defaults() {
        let c = SimulationConfig::from_toml_str(GOOD).unwrap();
        assert_eq!(c.seed, 42);
        assert_eq!(c.tick_interval(), Duration::from_millis(250));
        assert_relative_eq!(c.speed, 120.0);
        assert_relative_eq!(c.vol_process.mean_reversion, 5.0);
        assert_relative_eq!(c.vol_process.vol_of_vol, 0.9);
        assert_relative_eq!(c.vol_process.spot_vol_correlation, -0.7);
        assert_eq!(c.start_date, None);
    }

    #[test]
    fn rejects_broken_configs() {
        let cases = [
            GOOD.replace("tick_interval_ms = 250", "tick_interval_ms = 0"),
            GOOD.replace("speed = 120", "speed = 0"),
            GOOD.replace("speed = 120", "speed = -1"),
            GOOD.replace("speed = 120", "speed = nan"),
            GOOD.replace("drift = 0.0", "drift = inf"),
            GOOD.replace("mean_reversion = 5.0", "mean_reversion = 0.0"),
            GOOD.replace("vol_of_vol = 0.9", "vol_of_vol = -0.1"),
            GOOD.replace("spot_vol_correlation = -0.7", "spot_vol_correlation = 1.5"),
            GOOD.replace(
                "spot_vol_correlation = -0.7",
                "spot_vol_correlation = -1.01",
            ),
            GOOD.replace("seed = 42", "seed = -1"),
            GOOD.replace("seed = 42", "seed = 42\nstart_date = \"not a date\""),
            GOOD.replace("[vol_process]", "extra = 1\n[vol_process]"),
            GOOD.replace("mean_reversion = 5.0\n", ""),
        ];
        for (i, text) in cases.iter().enumerate() {
            assert!(SimulationConfig::from_toml_str(text).is_err(), "case {i}");
        }
    }

    #[test]
    fn boundary_correlations_and_zero_vol_of_vol_are_allowed() {
        let text = GOOD
            .replace("vol_of_vol = 0.9", "vol_of_vol = 0.0")
            .replace("spot_vol_correlation = -0.7", "spot_vol_correlation = -1.0");
        assert!(SimulationConfig::from_toml_str(&text).is_ok());
    }

    #[test]
    fn missing_file_is_a_read_error() {
        let err = SimulationConfig::load(Path::new("/definitely/not/here.toml")).unwrap_err();
        assert!(matches!(err, ConfigError::Read { .. }));
    }

    #[test]
    fn default_start_is_the_first_open_after_the_last_close() {
        let c = SimulationConfig::from_toml_str(GOOD).unwrap();
        let friday = NaiveDate::from_ymd_opt(2026, 9, 25).unwrap();
        assert_eq!(
            c.start_time(&seed_as_of(friday)),
            Utc.with_ymd_and_hms(2026, 9, 28, 13, 30, 0).unwrap()
        );
    }

    #[test]
    fn start_date_overrides_the_default() {
        let seed = seed_as_of(NaiveDate::from_ymd_opt(2026, 9, 25).unwrap());
        let with = |date: &str| {
            SimulationConfig::from_toml_str(
                &GOOD.replace("seed = 42", &format!("seed = 42\nstart_date = \"{date}\"")),
            )
            .unwrap()
            .start_time(&seed)
        };
        // A weekday opens that day. 09:30 EDT is 13:30 UTC.
        assert_eq!(
            with("2026-10-07"),
            Utc.with_ymd_and_hms(2026, 10, 7, 13, 30, 0).unwrap()
        );
        // A Saturday opens on Monday. After the autumn change the open is 14:30 UTC.
        assert_eq!(
            with("2026-10-31"),
            Utc.with_ymd_and_hms(2026, 11, 2, 14, 30, 0).unwrap()
        );
        // The override wins even when it is before the seed date.
        assert_eq!(
            with("2026-01-05"),
            Utc.with_ymd_and_hms(2026, 1, 5, 14, 30, 0).unwrap()
        );
    }
}
