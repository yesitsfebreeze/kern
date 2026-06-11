use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Checkpoint {
	pub label: String,
	pub elapsed_ms: f64,
}

#[derive(Debug, Clone)]
pub struct Profile {
	pub name: String,
	pub checkpoints: Vec<Checkpoint>,
	pub total_ms: f64,
}

impl Profile {
	pub fn checkpoint(&self, label: &str) -> Option<f64> {
		self.checkpoints.iter().find(|c| c.label == label).map(|c| c.elapsed_ms)
	}
}

pub struct Profiler {
	name: String,
	start: Instant,
	checkpoints: Vec<(String, Instant)>,
}

impl Profiler {
	pub fn new(name: impl Into<String>) -> Self {
		Self { name: name.into(), start: Instant::now(), checkpoints: vec![] }
	}

	pub fn checkpoint(&mut self, label: impl Into<String>) {
		self.checkpoints.push((label.into(), Instant::now()));
	}

	pub fn finish(self) -> Profile {
		let total = self.start.elapsed().as_secs_f64() * 1000.0;
		let mut checkpoints = Vec::new();

		let mut prev = self.start;
		for (label, t) in self.checkpoints {
			let elapsed = t.duration_since(prev).as_secs_f64() * 1000.0;
			checkpoints.push(Checkpoint { label, elapsed_ms: elapsed });
			prev = t;
		}

		Profile { name: self.name, checkpoints, total_ms: total }
	}
}

impl std::fmt::Display for Profile {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let stages = self
			.checkpoints
			.iter()
			.map(|c| format!("{}={:.1}ms", c.label, c.elapsed_ms))
			.collect::<Vec<_>>()
			.join(" ");
		write!(f, "{}: {} [total {:.1}ms]", self.name, stages, self.total_ms)
	}
}

/// Render a set of profiles as an aligned ASCII timeline. Bars are scaled to
/// the slowest profile; stage segments cycle fill characters so a bar's
/// composition stays visible, and the per-stage numbers follow in parens.
pub fn render_timeline(profiles: &[Profile], width: usize) -> String {
	const FILLS: [char; 4] = ['█', '▓', '▒', '░'];
	let max = profiles.iter().map(|p| p.total_ms).fold(0.0_f64, f64::max);
	if max <= 0.0 || profiles.is_empty() {
		return String::new();
	}
	let name_w = profiles.iter().map(|p| p.name.chars().count()).max().unwrap_or(0);

	let mut out = String::new();
	for p in profiles {
		let mut bar = String::new();
		if p.checkpoints.is_empty() {
			let n = ((p.total_ms / max) * width as f64).round() as usize;
			bar.extend(std::iter::repeat('█').take(n.max(1)));
		} else {
			for (i, c) in p.checkpoints.iter().enumerate() {
				// Floor a positive stage to at least one cell: rounding alone
				// drops a small-but-nonzero stage (e.g. the second-slowest) to a
				// 0-width, invisible bar. A genuinely zero stage stays empty.
				let n = if c.elapsed_ms > 0.0 {
					(((c.elapsed_ms / max) * width as f64).round() as usize).max(1)
				} else {
					0
				};
				bar.extend(std::iter::repeat(FILLS[i % FILLS.len()]).take(n));
			}
			if bar.is_empty() {
				bar.push('█');
			}
		}
		out.push_str(&format!("{:<name_w$}  {:>9.1}ms  {}", p.name, p.total_ms, bar));
		if !p.checkpoints.is_empty() {
			let stages = p
				.checkpoints
				.iter()
				.map(|c| format!("{}={:.1}ms", c.label, c.elapsed_ms))
				.collect::<Vec<_>>()
				.join(" ");
			out.push_str(&format!("  ({stages})"));
		}
		out.push('\n');
	}
	out
}

/// Macro to instrument a code block with timing.
/// Usage: profile_block!("name", { /* code */ })
#[macro_export]
macro_rules! profile_block {
	($name:expr, $code:block) => {{
		let mut prof = $crate::profile::Profiler::new($name);
		let result = { $code };
		// Record the block body as a single stage so `finish()` yields a
		// non-empty profile. The previous version finished the Profiler with
		// ZERO checkpoints (and held a dead `start` Instant that was never read),
		// so the logged profile carried no stage timing at all.
		prof.checkpoint("body");
		tracing::debug!("{}", prof.finish());
		result
	}};
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::thread::sleep;
	use std::time::Duration;

	#[test]
	fn profiler_records_checkpoints() {
		let mut prof = Profiler::new("test");
		sleep(Duration::from_millis(10));
		prof.checkpoint("stage1");
		sleep(Duration::from_millis(5));
		prof.checkpoint("stage2");
		sleep(Duration::from_millis(5));

		let profile = prof.finish();

		assert_eq!(profile.name, "test");
		assert_eq!(profile.checkpoints.len(), 2);
		assert_eq!(profile.checkpoints[0].label, "stage1");
		assert_eq!(profile.checkpoints[1].label, "stage2");

		// Each checkpoint should have elapsed time >= its sleep duration (within reason)
		assert!(profile.checkpoints[0].elapsed_ms >= 8.0, "stage1 took {}", profile.checkpoints[0].elapsed_ms);
		assert!(profile.checkpoints[1].elapsed_ms >= 3.0, "stage2 took {}", profile.checkpoints[1].elapsed_ms);
		assert!(profile.total_ms >= 20.0, "total took {}", profile.total_ms);
	}

	#[test]
	fn profile_display_formats_correctly() {
		let prof = Profile {
			name: "test".to_string(),
			checkpoints: vec![
				Checkpoint { label: "stage1".to_string(), elapsed_ms: 1.5 },
				Checkpoint { label: "stage2".to_string(), elapsed_ms: 2.3 },
			],
			total_ms: 3.8,
		};

		let output = prof.to_string();
		assert!(output.contains("test:"), "output should contain name");
		assert!(output.contains("stage1=1.5ms"), "output should contain stage1");
		assert!(output.contains("stage2=2.3ms"), "output should contain stage2");
		assert!(output.contains("total 3.8ms"), "output should contain total");
	}

	#[test]
	fn render_timeline_scales_and_lists_stages() {
		let profiles = vec![
			Profile {
				name: "fast".to_string(),
				checkpoints: vec![],
				total_ms: 10.0,
			},
			Profile {
				name: "slow".to_string(),
				checkpoints: vec![
					Checkpoint { label: "a".to_string(), elapsed_ms: 60.0 },
					Checkpoint { label: "b".to_string(), elapsed_ms: 40.0 },
				],
				total_ms: 100.0,
			},
		];

		let out = render_timeline(&profiles, 20);
		let lines: Vec<&str> = out.lines().collect();
		assert_eq!(lines.len(), 2);
		assert!(lines[0].contains("fast"), "first row names fast op");
		assert!(lines[1].contains("a=60.0ms b=40.0ms"), "stages listed: {out}");
		// slow bar fills the full width, fast bar is ~2 cells
		let slow_bar: usize = lines[1].chars().filter(|c| "█▓▒░".contains(*c)).count();
		let fast_bar: usize = lines[0].chars().filter(|c| *c == '█').count();
		assert_eq!(slow_bar, 20, "slow bar spans full width: {out}");
		assert_eq!(fast_bar, 2, "fast bar scaled to 10%: {out}");
	}

	#[test]
	fn render_timeline_empty_and_zero() {
		assert_eq!(render_timeline(&[], 20), "");
		let zero = vec![Profile { name: "z".to_string(), checkpoints: vec![], total_ms: 0.0 }];
		assert_eq!(render_timeline(&zero, 20), "");
	}

	#[test]
	fn render_timeline_tiny_nonzero_stage_gets_at_least_one_cell() {
		// `tiny` is 0.4/100*20 = 0.08 -> round() = 0 cells without the min-1
		// floor; with it, the stage (FILLS[1] = '▓') must still appear once.
		let profiles = vec![Profile {
			name: "p".to_string(),
			checkpoints: vec![
				Checkpoint { label: "big".to_string(), elapsed_ms: 99.0 },
				Checkpoint { label: "tiny".to_string(), elapsed_ms: 0.4 },
			],
			total_ms: 100.0,
		}];
		let out = render_timeline(&profiles, 20);
		assert!(out.contains('▓'), "tiny non-zero stage must render >=1 cell: {out}");
	}

	#[test]
	fn profile_block_macro_returns_block_value_and_expands_clean() {
		// Expanding the macro guards against the old dead-`start`/no-checkpoint
		// regression; the macro must evaluate to the block's value.
		let x = crate::profile_block!("unit", {
			let a = 2;
			a + 3
		});
		assert_eq!(x, 5);
	}
}
