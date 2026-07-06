use serde_json;

/// Formats a preview of new file content, pretty-printing JSON lines.
pub(super) fn format_file_content_preview(new_content: &str) -> String {
    const MAX_LINES_TO_LOG: usize = 5;
    let lines: Vec<_> = new_content.lines().collect();
    let total_lines = lines.len();

    let mut formatted_lines = Vec::new();
    for line in lines.iter().take(MAX_LINES_TO_LOG) {
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
            // It's a valid JSON line. Pretty-print it.
            let pretty_json =
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| line.trim().to_string());

            // Indent the entire pretty-printed block.
            let indented_block = pretty_json
                .lines()
                .map(|l| format!("    {}", l))
                .collect::<Vec<_>>()
                .join("\n");
            formatted_lines.push(indented_block);
        } else {
            // Not valid JSON, just print the line
            formatted_lines.push(format!("    > {}", line.trim_end()));
        }
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