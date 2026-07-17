//! WSL2 NAT networking: the loopback default resolves inside the WSL VM, not the
//! Windows host where Ollama listens, so embeds fail silently and the graph stays
//! empty. Rewrite the DEFAULT loopback URL to the host gateway, but only when
//! loopback is genuinely dead so mirrored-mode WSL2 and in-distro Ollama are
//! untouched. An explicitly configured URL is never second-guessed.

use std::io::BufRead as _;
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const PROBE_TIMEOUT: Duration = Duration::from_millis(300);

// Reads `/proc/version`, not `WSL_DISTRO_NAME` (unset under some service managers,
// and settable by anyone). WSL1/WSL2 both stamp the kernel release string.
pub fn in_wsl() -> bool {
	std::fs::read_to_string("/proc/version")
		.map(|v| {
			let v = v.to_ascii_lowercase();
			v.contains("microsoft") || v.contains("wsl")
		})
		.unwrap_or(false)
}

// Windows host = the default-route gateway, not `/etc/resolv.conf`'s nameserver:
// that diverges under `generateResolvConf=false` or custom DNS, so route wins.
fn host_gateway() -> Option<String> {
	let f = std::fs::File::open("/proc/net/route").ok()?;
	for line in std::io::BufReader::new(f)
		.lines()
		.skip(1)
		.map_while(Result::ok)
	{
		let mut cols = line.split_whitespace();
		let (_iface, dest, gw) = (cols.next()?, cols.next()?, cols.next()?);
		// Default route: destination 0.0.0.0; gateway is little-endian hex.
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

fn is_loopback_url(url: &str) -> bool {
	split_host_port(url)
		.map(|(h, _)| h == "localhost" || h == "127.0.0.1" || h == "::1" || h == "[::1]")
		.unwrap_or(false)
}

// Probe loopback FIRST so a mirrored-mode WSL2 (or a WSL distro running its own
// Ollama) keeps loopback and never pays a rewrite it did not need.
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
		assert!(!is_loopback_url("http://172.27.176.1:11434"));
		assert!(!is_loopback_url("https://api.openai.com/v1"));
	}

	#[test]
	fn a_non_loopback_url_is_never_rewritten_even_inside_wsl() {
		assert_eq!(resolve_loopback("http://172.27.176.1:11434"), None);
		assert_eq!(resolve_loopback("https://api.openai.com/v1"), None);
	}

	#[test]
	fn a_dead_port_resolves_to_no_rewrite() {
		assert_eq!(resolve_loopback("http://localhost:1"), None);
	}

	#[test]
	fn host_gateway_parses_the_route_table_or_declines() {
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
