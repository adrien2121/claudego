use std::io::Write;

const ORDINARY: &[u8] = b"{\"type\":\"assistant\"}\n";
const INVALID: &[u8] = b"not json\n";
const UNKNOWN: &[u8] = b"{\"unknown\":true}\n";
const SESSION: &[u8] =
    b"{\"type\":\"system\",\"session_id\":\"11111111-1111-1111-1111-111111111111\"}\n";
const RATE_LIMIT: &[u8] = b"{\"type\":\"error\",\"timestamp\":\"2099-07-09T10:00:00-04:00\",\"error\":\"rate_limit\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Claude limit reached; resets 5:30pm\"}]}}\n";

fn write_fragmented(out: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    for chunk in bytes.chunks(7) {
        out.write_all(chunk)?;
        out.flush()?;
        std::thread::yield_now();
    }
    Ok(())
}

fn scenario_from<'a>(args: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    args.into_iter()
        .last()
        .filter(|scenario| matches!(*scenario, "stream-signal" | "no-stream-signal"))
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let scenario = scenario_from(args.iter().map(String::as_str));
    let Some(scenario) = scenario else {
        eprintln!("usage: stream-stress-child <stream-signal|no-stream-signal>");
        std::process::exit(2);
    };

    let mut out = std::io::stdout().lock();
    write_fragmented(&mut out, ORDINARY)?;
    write_fragmented(&mut out, INVALID)?;
    write_fragmented(&mut out, UNKNOWN)?;
    write_fragmented(&mut out, SESSION)?;
    if scenario == "stream-signal" {
        write_fragmented(&mut out, RATE_LIMIT)?;
    }
    for index in 0..1_100 {
        write_fragmented(
            &mut out,
            format!("{{\"type\":\"overload\",\"index\":{index}}}\n").as_bytes(),
        )?;
    }
    write_fragmented(&mut out, b"{\"incomplete\":true}")?;
    eprintln!("READY");
    // macOS FSEvents delivery can exceed the output-hot threshold by several seconds.
    // Keep the child alive long enough for the real watcher to observe the test append.
    std::thread::sleep(std::time::Duration::from_secs(10));
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn accepts_scenario_after_claude_style_flags() {
        let args = [
            "claude",
            "-p",
            "--output-format",
            "stream-json",
            "no-stream-signal",
        ];

        assert_eq!(super::scenario_from(args), Some("no-stream-signal"));
    }
}
