use serde_json;

/// Formats a preview of new file content, pretty-printing JSON lines.
pub(super) fn format_file_content_preview(new_content: &str) -> String {
    const MAX_LINES_TO_LOG: usize = 5;
    let lines: Vec<_> = new_content.lines().collect();
    let total_lines = lines.len();

    let mut formatted_lines = Vec::new();
    for line in lines.iter().take(MAX_LINES_TO_LOG) {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            continue;
        }

        // Optimization: Only attempt to parse/pretty-print if it looks like a JSON object.
        // This avoids expensive parsing on every line of conversational text.
        if trimmed_line.starts_with('{') && trimmed_line.ends_with('}') {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed_line) {
                // It's a valid JSON line. Pretty-print it.
                let pretty_json =
                    serde_json::to_string_pretty(&value).unwrap_or_else(|_| trimmed_line.to_string());
                // Indent the entire pretty-printed block.
                let indented_block = pretty_json
                    .lines()
                    .map(|l| format!("    {}", l))
                    .collect::<Vec<_>>()
                    .join("\n");
                formatted_lines.push(indented_block);
                continue;
            }
        }
        // Not a valid/parsable JSON object, just print the line as-is.
        formatted_lines.push(format!("    > {}", line.trim_end()));
    }

    let mut preview = formatted_lines.join("\n");

    if total_lines > MAX_LINES_TO_LOG {
        preview.push_str(&format!(
            "\n    ... (and {} more lines)",
            total_lines - MAX_LINES_TO_LOG
        ));
    }

    preview
}