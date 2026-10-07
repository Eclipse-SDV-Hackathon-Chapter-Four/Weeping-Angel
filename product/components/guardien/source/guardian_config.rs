use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigurationFile {
    guardian: GuardianConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianConfig {
    pub evaluation_period_ms: u64,
    pub missing_packet_timeout_ms: u64,
    pub temperature: TemperatureConfig,
    pub soc: SocConfig,
    pub stuck: StuckConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemperatureConfig {
    pub absolute_min_c: f32,
    pub absolute_max_c: f32,
    pub reference_c: f32,
    pub hot_state_c: f32,
    pub spread: ThermalLimitConfig,
    pub hotspot: ThermalLimitConfig,
    pub dynamics: TemperatureDynamicsConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThermalLimitConfig {
    pub cold_c: f32,
    pub hot_c: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemperatureDynamicsConfig {
    pub heating_rate_c_per_s: HeatingRateConfig,
    pub cooling_rate_c_per_s: f32,
    pub soc_coupling: SocCouplingConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeatingRateConfig {
    pub cold: f32,
    pub hot: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocCouplingConfig {
    pub enabled: bool,
    pub gain_c_per_pp: f32,
    pub rate_cap_pp_per_s: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocConfig {
    pub min_percent: f32,
    pub max_percent: f32,
    pub max_step_pp: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StuckConfig {
    pub enabled: bool,
    pub window_samples: usize,
    pub flatness_epsilon_c: f32,
    pub temperature_excitation_c: f32,
    pub soc_excitation_pp: f32,
}

impl GuardianConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let contents = fs::read_to_string(path)
            .with_context(|| format!("read Guardian configuration {}", path.display()))?;
        Self::from_yaml_str(&contents)
            .with_context(|| format!("load Guardian configuration {}", path.display()))
    }

    pub fn from_yaml_str(contents: &str) -> Result<Self> {
        let file: ConfigurationFile =
            serde_yaml::from_str(contents).context("parse Guardian configuration YAML")?;
        file.guardian.validate()?;
        Ok(file.guardian)
    }

    pub fn evaluation_period(&self) -> Duration {
        Duration::from_millis(self.evaluation_period_ms)
    }

    pub fn missing_packet_timeout(&self) -> Duration {
        Duration::from_millis(self.missing_packet_timeout_ms)
    }

    pub fn validate(&self) -> Result<()> {
        if self.evaluation_period_ms == 0 {
            bail!("guardian.evaluation_period_ms must be greater than zero");
        }
        if self.missing_packet_timeout_ms == 0 {
            bail!("guardian.missing_packet_timeout_ms must be greater than zero");
        }

        let temperature = &self.temperature;
        require_finite("temperature.absolute_min_c", temperature.absolute_min_c)?;
        require_finite("temperature.absolute_max_c", temperature.absolute_max_c)?;
        require_finite("temperature.reference_c", temperature.reference_c)?;
        require_finite("temperature.hot_state_c", temperature.hot_state_c)?;
        if temperature.absolute_min_c >= temperature.absolute_max_c {
            bail!("temperature.absolute_min_c must be less than absolute_max_c");
        }
        if temperature.reference_c >= temperature.hot_state_c {
            bail!("temperature.reference_c must be less than hot_state_c");
        }

        validate_thermal_limit("temperature.spread", &temperature.spread)?;
        validate_thermal_limit("temperature.hotspot", &temperature.hotspot)?;

        let dynamics = &temperature.dynamics;
        let heating = &dynamics.heating_rate_c_per_s;
        require_finite(
            "temperature.dynamics.heating_rate_c_per_s.cold",
            heating.cold,
        )?;
        require_finite("temperature.dynamics.heating_rate_c_per_s.hot", heating.hot)?;
        if heating.hot <= 0.0 || heating.cold < heating.hot {
            bail!("heating rate must satisfy cold >= hot > 0");
        }
        require_positive(
            "temperature.dynamics.cooling_rate_c_per_s",
            dynamics.cooling_rate_c_per_s,
        )?;
        require_non_negative(
            "temperature.dynamics.soc_coupling.gain_c_per_pp",
            dynamics.soc_coupling.gain_c_per_pp,
        )?;
        require_positive(
            "temperature.dynamics.soc_coupling.rate_cap_pp_per_s",
            dynamics.soc_coupling.rate_cap_pp_per_s,
        )?;

        require_finite("soc.min_percent", self.soc.min_percent)?;
        require_finite("soc.max_percent", self.soc.max_percent)?;
        if self.soc.min_percent >= self.soc.max_percent {
            bail!("soc.min_percent must be less than soc.max_percent");
        }
        require_positive("soc.max_step_pp", self.soc.max_step_pp)?;

        if self.stuck.window_samples < 2 {
            bail!("stuck.window_samples must be at least 2");
        }
        require_non_negative("stuck.flatness_epsilon_c", self.stuck.flatness_epsilon_c)?;
        require_positive(
            "stuck.temperature_excitation_c",
            self.stuck.temperature_excitation_c,
        )?;
        require_positive("stuck.soc_excitation_pp", self.stuck.soc_excitation_pp)?;

        Ok(())
    }
}

fn validate_thermal_limit(name: &str, limit: &ThermalLimitConfig) -> Result<()> {
    require_non_negative(&format!("{name}.cold_c"), limit.cold_c)?;
    require_non_negative(&format!("{name}.hot_c"), limit.hot_c)?;
    if limit.cold_c < limit.hot_c {
        bail!("{name} must satisfy cold_c >= hot_c >= 0");
    }
    Ok(())
}

fn require_finite(name: &str, value: f32) -> Result<()> {
    if !value.is_finite() {
        bail!("{name} must be finite");
    }
    Ok(())
}

fn require_positive(name: &str, value: f32) -> Result<()> {
    require_finite(name, value)?;
    if value <= 0.0 {
        bail!("{name} must be greater than zero");
    }
    Ok(())
}

fn require_non_negative(name: &str, value: f32) -> Result<()> {
    require_finite(name, value)?;
    if value < 0.0 {
        bail!("{name} must be non-negative");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = include_str!("../../../config/battery_guardian.yaml");

    #[test]
    fn supplied_configuration_is_valid() {
        GuardianConfig::from_yaml_str(CONFIG).expect("configuration should be valid");
    }

    #[test]
    fn missing_mandatory_parameter_fails() {
        let yaml = CONFIG.replace("  evaluation_period_ms: 100\n", "");
        assert!(GuardianConfig::from_yaml_str(&yaml).is_err());
    }

    #[test]
    fn inconsistent_temperature_range_fails() {
        let yaml = CONFIG.replace("absolute_min_c: -30.0", "absolute_min_c: 70.0");
        assert!(GuardianConfig::from_yaml_str(&yaml).is_err());
    }
}
