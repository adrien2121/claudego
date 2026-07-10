use std::io::Write;

/// Creates a formatted preview of new file content as a string.
pub(super) fn create_content_preview(new_content: &str) -> String {
    let mut buffer = Vec::new();
    // Writing to a Vec<u8> buffer should not fail, so we can ignore the result.
    let _ = write_preview_to_buffer(&mut buffer, new_content);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Helper that writes a formatted preview of new file content to a writer.
fn write_preview_to_buffer(writer: &mut dyn Write, new_content: &str) -> std::io::Result<()> {
    const MAX_LINES_TO_LOG: usize = 5;
    let lines: Vec<_> = new_content.lines().collect();
    let total_lines = lines.len();

    for line in lines.iter().take(MAX_LINES_TO_LOG) {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        writeln!(writer, "    > {}", line.trim_end())?;
    }

    if total_lines > MAX_LINES_TO_LOG {
        writeln!(
            writer,
            "    ... (and {} more lines)",
            total_lines - MAX_LINES_TO_LOG
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::create_content_preview;

    #[test]
    fn preview_keeps_json_compact() {
        let preview = create_content_preview(r#"{"type":"event","nested":{"value":1}}"#);

        assert_eq!(
            preview,
            "    > {\"type\":\"event\",\"nested\":{\"value\":1}}\n"
        );
    }
}
