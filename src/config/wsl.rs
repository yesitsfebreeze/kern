//! Resolving the default Ollama URL under WSL2.
//!
//! kern's promise is that a stock install just works against a local Ollama.
//! Under WSL2 with the default (NAT) networking mode that promise silently
//! broke: Ollama runs as a Windows host process, the loopback default
//! (`http://localhost:11434`) resolves inside the WSL VM where nothing is
//! listening, every embed call fails as a transient connect error, and ingest
//! leaves the job spooled forever. The failure is invisible — no crash, no
//! error surfaced to the user, just a graph that stays empty. (Measured on this
//! machine: 13 daemons, weeks of uptime, zero thoughts each.)
//!
//! WSL2 in *mirrored* networking mode DOES reach the host over loopback, and a
//! Linux box running its own Ollama obviously does too, so this must never be a
//! blanket "on WSL, rewrite to the gateway" rule. Hence: probe loopback first
//! and only fall back to the host gateway when loopback is genuinely dead. This
//! is a rewrite of the DEFAULT only — an explicitly configured URL is the
//! user's decision and is never second-guessed.

use std::io::BufRead as _;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// How long to wait for the loopback/gateway TCP probes. Both ends are local
/// (a VM loopback and the host across a virtual switch), so a live Ollama
/// answers in single-digit milliseconds; this only bounds the dead case.
const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

/// Whether this process is running inside WSL. WSL1 and WSL2 both stamp the
/// kernel release string, which is why this reads `/proc/version` rather than
/// trusting `WSL_DISTRO_NAME` (unset under some service managers and settable
/// by anyone).
pub fn in_wsl() -> bool {
	std::fs::read_to_string("/proc/version")
		.map(|v| {
			let v = v.to_ascii_lowercase();
			v.contains("microsoft") || v.contains("wsl")
		})
		.unwrap_or(false)
}

/// The Windows host address as seen from a NAT-mode WSL2 VM: the default
/// route's gateway. `/etc/resolv.conf`'s nameserver usually matches it, but not
/// when `generateResolvConf=false` or a custom DNS is set, so the route table
/// is the authority.
fn host_gateway() -> Option<String> {
	let f = std::fs::File::open("/proc/net/route").ok()?;
	for line in std::io::BufReader::new(f)
		.lines()
		.skip(1)
		.map_while(Result::ok)
	{
		let mut cols = line.split_whitespace();
		let (_iface, dest, gw) = (cols.next()?, cols.next()?, cols.next()?);
		// The default route: destination 0.0.0.0. Gateway is little-endian hex.
		if dest != "00000000" {
			continue;
		}
		let raw = u32::from_str_radix(gw, 16).ok()?;
		let o = raw.to_le_bytes();
		if raw != 0 {
			return Some(format!("{}.{}.{}.{}", o[0], o[1], o[2], o[3]));
		}
	}
	None
}

fn port_open(host: &str, port: u16) -> bool {
	use std::net::ToSocketAddrs as _;
	let Ok(addrs) = (host, port).to_socket_addrs() else {
		return false;
	};
	addrs
		.filter_map(|a: SocketAddr| TcpStream::connect_timeout(&a, PROBE_TIMEOUT).ok())
		.next()
		.is_some()
}

/// Split a `http://host:port` URL into (host, port), port defaulting to 80.
/// Deliberately tiny — this only ever sees kern's own loopback defaults, and
/// pulling in a URL parser for that would be the tail wagging the dog.
fn split_host_port(url: &str) -> Option<(String, u16)> {
	let rest = url
		.strip_prefix("http://")
		.or_else(|| url.strip_prefix("https://"))?;
	let authority = rest.split('/').next()?;
	match authority.rsplit_once(':') {
		Some((h, p)) => Some((h.to_string(), p.parse().ok()?)),
		None => Some((authority.to_string(), 80)),
	}
}

/// Whether `url` points at loopback — the only shape this module rewrites.
fn is_loopback_url(url: &str) -> bool {
	split_host_port(url)
		.map(|(h, _)| h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]")
		.unwrap_or(false)
}

/// Rewrite a loopback `url` to the WSL host gateway when — and only when — all
/// of: we are inside WSL, the URL is loopback, loopback is NOT listening, and
/// the gateway IS. Returns `None` when the URL should be left exactly as-is,
/// which is the common case on every non-WSL machine.
///
/// Ordering matters: loopback is probed FIRST so a mirrored-mode WSL2 (or a WSL
/// distro running its own Ollama) keeps using loopback and never pays a rewrite
/// it did not need.
pub fn resolve_loopback(url: &str) -> Option<String> {
	if !in_wsl() || !is_loopback_url(url) {
		return None;
	}
	let (_, port) = split_host_port(url)?;
	if port_open("127.0.0.1", port) {
		return None;
	}
	let gw = host_gateway()?;
	if !port_open(&gw, port) {
		return None;
	}
	Some(format!("http://{gw}:{port}"))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn splits_host_and_port_with_a_default_of_80() {
		assert_eq!(
			split_host_port("http://localhost:11434"),
			Some(("localhost".into(), 11434))
		);
		assert_eq!(
			split_host_port("http://127.0.0.1:8000/v1"),
			Some(("127.0.0.1".into(), 8000))
		);
		assert_eq!(
			split_host_port("http://example.com"),
			Some(("example.com".into(), 80))
		);
		assert_eq!(split_host_port("not-a-url"), None);
	}

	#[test]
	fn only_loopback_urls_are_rewrite_candidates() {
		assert!(is_loopback_url("http://localhost:11434"));
		assert!(is_loopback_url("http://127.0.0.1:11434"));
		// A real host — the user's explicit choice, never touched.
		assert!(!is_loopback_url("http://172.27.176.1:11434"));
		assert!(!is_loopback_url("https://api.openai.com/v1"));
	}

	/// The guard that keeps this from firing on normal Linux/macOS boxes.
	#[test]
	fn a_non_loopback_url_is_never_rewritten_even_inside_wsl() {
		assert_eq!(resolve_loopback("http://172.27.176.1:11434"), None);
		assert_eq!(resolve_loopback("https://api.openai.com/v1"), None);
	}

	/// Nothing listens on this port anywhere, so on a non-WSL host the answer is
	/// None (not-WSL guard) and inside WSL it is None too (no gateway listener).
	/// Either way: no rewrite, no panic.
	#[test]
	fn a_dead_port_resolves_to_no_rewrite() {
		assert_eq!(resolve_loopback("http://localhost:1"), None);
	}

	#[test]
	fn host_gateway_parses_the_route_table_or_declines() {
		// Must never panic on whatever /proc/net/route this machine has; on a
		// routed box it yields a dotted quad.
		if let Some(gw) = host_gateway() {
			assert_eq!(
				gw.split('.').count(),
				4,
				"gateway must be a dotted quad: {gw}"
			);
			assert!(gw.split('.').all(|o| o.parse::<u8>().is_ok()));
		}
	}
}
