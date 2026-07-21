use crate::base::constants::{AGENT_SOURCE, USER_SOURCE};

pub const CONF_MIN: f64 = 0.0;
pub const CONF_MAX: f64 = 1.0;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidateError {
	#[error("conf {0} out of range [0.0..=1.0]")]
	ConfOutOfRange(f64),
	#[error("fact-tier conf requires trusted source (got source={0:?})")]
	FactFromUntrustedSource(String),
}

pub fn validate_conf(conf: f64) -> Result<f64, ValidateError> {
	if conf.is_nan() || !(CONF_MIN..=CONF_MAX).contains(&conf) {
		return Err(ValidateError::ConfOutOfRange(conf));
	}
	Ok(conf)
}

pub fn validate_fact_source(source: &str) -> Result<(), ValidateError> {
	if source == USER_SOURCE || source == AGENT_SOURCE {
		Ok(())
	} else {
		Err(ValidateError::FactFromUntrustedSource(source.to_string()))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn conf_out_of_range_rejected_high() {
		assert!(matches!(
			validate_conf(1.5),
			Err(ValidateError::ConfOutOfRange(_))
		));
	}

	#[test]
	fn conf_out_of_range_rejected_low() {
		assert!(matches!(
			validate_conf(-0.01),
			Err(ValidateError::ConfOutOfRange(_))
		));
	}

	#[test]
	fn conf_out_of_range_rejected_nan() {
		assert!(matches!(
			validate_conf(f64::NAN),
			Err(ValidateError::ConfOutOfRange(_))
		));
	}

	#[test]
	fn conf_inclusive_bounds_accepted() {
		assert_eq!(validate_conf(0.0), Ok(0.0));
		assert_eq!(validate_conf(1.0), Ok(1.0));
		assert_eq!(validate_conf(0.5), Ok(0.5));
	}

	#[test]
	fn fact_source_rejects_untrusted() {
		assert!(matches!(
			validate_fact_source("stranger"),
			Err(ValidateError::FactFromUntrustedSource(_))
		));
	}

	#[test]
	fn fact_source_allows_trusted() {
		assert!(validate_fact_source(AGENT_SOURCE).is_ok());
		assert!(validate_fact_source(USER_SOURCE).is_ok());
	}
}
