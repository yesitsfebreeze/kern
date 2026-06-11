use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use crate::error::McpError;

pub trait Transport: Send {
	fn reader(&mut self) -> &mut dyn Read;
	fn writer(&mut self) -> &mut dyn Write;
	fn kill(&mut self) -> std::io::Result<()>;
}

pub struct ChildStdio {
	child: Child,
	reader: BufReader<ChildStdout>,
	writer: ChildStdin,
}

impl ChildStdio {
	pub fn spawn(program: &str, args: &[&str]) -> Result<Self, McpError> {
		let mut child = Command::new(program)
			.args(args)
			.stdin(Stdio::piped())
			.stdout(Stdio::piped())
			.stderr(Stdio::piped())
			.spawn()?;
		let stdout = child
			.stdout
			.take()
			.ok_or_else(|| McpError::Protocol("child stdout missing".into()))?;
		let stdin = child
			.stdin
			.take()
			.ok_or_else(|| McpError::Protocol("child stdin missing".into()))?;
		if let Some(stderr) = child.stderr.take() {
			let source = child_log_source(program);
			std::thread::Builder::new()
				.name(format!("mcp-stderr-{source}"))
				.spawn(move || forward_child_stderr(stderr, &source))
				.ok();
		}
		Ok(Self {
			child,
			reader: BufReader::new(stdout),
			writer: stdin,
		})
	}
}

fn child_log_source(program: &str) -> String {
	let path = std::path::Path::new(program);
	let stem = path
		.file_stem()
		.and_then(|s| s.to_str())
		.unwrap_or(program);
	stem.to_string()
}

fn forward_child_stderr<R: std::io::Read>(stderr: R, source: &str) {
	let reader = BufReader::new(stderr);
	for line in reader.lines().map_while(Result::ok) {
		let trimmed = line.trim_end();
		if trimmed.is_empty() {
			continue;
		}
		let level = classify_child_log_level(trimmed);
		logsink::log(level, source, trimmed);
	}
}

fn classify_child_log_level(line: &str) -> logsink::Level {
	// Case-insensitive so a child logging "error", "Error" or "ERROR" all map to
	// Error (previously only upper-case "ERROR" / lower-case "error:" matched, so
	// "Error: boom" slipped through as Info).
	let upper = line.to_ascii_uppercase();
	if upper.contains("ERROR") {
		logsink::Level::Error
	} else if upper.contains("WARN") {
		logsink::Level::Warn
	} else {
		logsink::Level::Info
	}
}

impl Transport for ChildStdio {
	fn reader(&mut self) -> &mut dyn Read {
		&mut self.reader
	}
	fn writer(&mut self) -> &mut dyn Write {
		&mut self.writer
	}
	fn kill(&mut self) -> std::io::Result<()> {
		match self.child.try_wait()? {
			Some(_) => Ok(()),
			None => {
				let _ = self.child.kill();
				let _ = self.child.wait();
				Ok(())
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use logsink::Level;

	#[test]
	fn classify_child_log_level_is_case_insensitive_with_error_priority() {
		assert!(matches!(classify_child_log_level("ERROR: boom"), Level::Error));
		assert!(matches!(classify_child_log_level("Error happened"), Level::Error));
		assert!(matches!(classify_child_log_level("an error: detail"), Level::Error));
		assert!(matches!(classify_child_log_level("WARN: heads up"), Level::Warn));
		assert!(matches!(classify_child_log_level("a Warning: x"), Level::Warn));
		assert!(matches!(classify_child_log_level("just some info"), Level::Info));
		// Error wins over warn when both appear.
		assert!(matches!(classify_child_log_level("WARN then ERROR"), Level::Error));
	}

	#[test]
	fn child_log_source_is_the_file_stem() {
		assert_eq!(child_log_source("/usr/bin/foo-server"), "foo-server");
		assert_eq!(child_log_source("bar.exe"), "bar");
		assert_eq!(child_log_source("plain"), "plain");
	}

	#[test]
	fn child_stdio_round_trips_a_line_through_an_echo_child() {
		// A trivial echo child: read one line from stdin, write it to stdout, exit.
		// `cat` on unix; a one-line PowerShell readline on windows — both portable
		// enough to verify the spawn -> write -> read -> kill cycle.
		#[cfg(unix)]
		let mut t = ChildStdio::spawn("cat", &[]).expect("spawn cat");
		#[cfg(windows)]
		let mut t = ChildStdio::spawn(
			"powershell",
			&["-NoProfile", "-Command", "$l = [Console]::In.ReadLine(); [Console]::Out.WriteLine($l)"],
		)
		.expect("spawn powershell echo");

		t.writer().write_all(b"hello world\n").unwrap();
		t.writer().flush().unwrap();

		let mut line = String::new();
		BufRead::read_line(&mut BufReader::new(t.reader()), &mut line).unwrap();
		assert_eq!(line.trim_end(), "hello world", "stdin line echoed back on stdout");

		t.kill().unwrap();
	}
}
