use serde_json;
use std::io::Write;

/// Creates a formatted preview of new file content as a string.
pub(super) fn create_content_preview(new_content: &str) -> String {
    let mut buffer = Vec::new();
    // Writing to a Vec<u8> buffer should not fail, so we can ignore the result.
    let _ = write_preview_to_buffer(&mut buffer, new_content);
    String::from_utf8_lossy(&buffer).into_owned()
}

/// Helper that writes a formatted preview of new file content to a writer.
fn write_preview_to_buffer(
    writer: &mut dyn Write,
    new_content: &str,
) -> std::io::Result<()> {
    const MAX_LINES_TO_LOG: usize = 5;
    let lines: Vec<_> = new_content.lines().collect();
    let total_lines = lines.len();

    for line in lines.iter().take(MAX_LINES_TO_LOG) {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        if trimmed_line.starts_with('{') && trimmed_line.ends_with('}') {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed_line) {
                let pretty_json =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed_line.to_string());

                // Write the indented block line by line
                for json_line in pretty_json.lines() {
                    writeln!(writer, "    {}", json_line)?;
                }
                continue;
            }
        }
        // Not a valid/parsable JSON object, just write the line as-is.
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