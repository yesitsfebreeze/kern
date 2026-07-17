//! `KernRpcClient::connect_local` — dials the per-cwd kern endpoint with the
//! JSON-envelope codec. No port file: the endpoint resolver IS the coordination.

use std::time::Duration;

use crate::typed::{connect_kern, AdapterError, Channel, Endpoint, JsonEnvelopeCodec};

use super::svc::KernRpcClient;

/// Connect attempts before giving up — absorbs the daemon-start race (a client
/// launched alongside `kern --daemon` may dial before the listener is up).
pub const RETRIES: u32 = 5;
/// Default base delay between connect attempts, in milliseconds (jittered at use).
pub const RETRY_DELAY_MS: u64 = 100;

impl KernRpcClient<JsonEnvelopeCodec> {
	/// Connect to a kern singleton at the per-cwd endpoint. Caller is expected to
	/// run on a tokio runtime.
	pub async fn connect_local() -> Result<Self, AdapterError> {
		Self::connect_endpoint(&Endpoint::kern()).await
	}

	/// Connect at an explicit endpoint with the default retry budget
	/// ([`RETRIES`] / [`RETRY_DELAY_MS`]).
	pub async fn connect_endpoint(endpoint: &Endpoint) -> Result<Self, AdapterError> {
		Self::connect_endpoint_with_retry(endpoint, RETRIES, Duration::from_millis(RETRY_DELAY_MS))
			.await
	}

	/// Connect, retrying up to `retries` times with a [`jittered`] `base_delay`
	/// between attempts.
	pub async fn connect_endpoint_with_retry(
		endpoint: &Endpoint,
		retries: u32,
		base_delay: Duration,
	) -> Result<Self, AdapterError> {
		let mut last_err: Option<AdapterError> = None;
		for _ in 0..retries {
			match connect_kern(endpoint).await {
				Ok(adapter) => {
					let channel = Channel::new(adapter, JsonEnvelopeCodec::new());
					return Ok(KernRpcClient::new(channel));
				}
				Err(e) => last_err = Some(e),
			}
			tokio::time::sleep(jittered(base_delay)).await;
		}
		Err(last_err.unwrap_or_else(|| AdapterError::Other("no endpoint".into())))
	}
}

/// Full-jitter into `[base/2, base]`; zero base stays zero. Entropy is
/// wall-clock sub-second nanos — no `rand` dependency.
fn jittered(base: Duration) -> Duration {
	let base_ms = base.as_millis() as u64;
	if base_ms == 0 {
		return base;
	}
	let nanos = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map(|d| d.subsec_nanos() as u64)
		.unwrap_or(0);
	let half = base_ms / 2;
	Duration::from_millis(half + (nanos % (half + 1)))
}

#[cfg(test)]
mod tests {
	use super::*;

	/// An endpoint that nothing is listening on, for exercising the failure path.
	fn bogus_endpoint() -> Endpoint {
		#[cfg(unix)]
		{
			Endpoint::Unix(std::path::PathBuf::from(
				"/nonexistent/kern-test-bogus.sock",
			))
		}
		#[cfg(windows)]
		{
			Endpoint::NamedPipe(r"\\.\pipe\kern-test-bogus-nonexistent".to_string())
		}
	}

	#[test]
	fn jittered_stays_within_half_to_full_and_zero_stays_zero() {
		assert_eq!(jittered(Duration::ZERO), Duration::ZERO);
		for _ in 0..64 {
			let d = jittered(Duration::from_millis(100));
			assert!(
				d >= Duration::from_millis(50) && d <= Duration::from_millis(100),
				"jitter must stay in [base/2, base], got {d:?}",
			);
		}
	}

	#[tokio::test]
	async fn connect_endpoint_gives_up_after_exhausting_retries() {
		let res =
			KernRpcClient::connect_endpoint_with_retry(&bogus_endpoint(), 3, Duration::from_millis(1))
				.await;
		assert!(
			res.is_err(),
			"no server at the endpoint -> Err after retries"
		);
	}
}
