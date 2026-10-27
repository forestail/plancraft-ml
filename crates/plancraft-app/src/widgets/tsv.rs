//! Bounded Excel-style clipboard text, including quoted tabs and multiline cells.

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_CELLS: usize = 1_000_000;
const MAX_COLS: usize = 256;

pub fn encode(rows: &[Vec<String>]) -> String {
    rows.iter()
        .map(|row| {
            row.iter()
                .map(
                    |value| {
                        if value.contains(['\t', '\r', '\n', '"']) { format!("\"{}\"", value.replace('"', "\"\"")) } else { value.clone() }
                    },
                )
                .collect::<Vec<_>>()
                .join("\t")
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

pub fn decode(text: &str) -> Result<Vec<Vec<String>>, String> {
    if text.len() > MAX_BYTES {
        return Err(crate::tr("Clipboard data is too large").into());
    }
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut at_start = true;
    let mut ended_row = false;
    let mut cells = 0usize;
    while let Some(ch) = chars.next() {
        ended_row = false;
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' && at_start {
            quoted = true;
            at_start = false;
        } else if matches!(ch, '\t' | '\r' | '\n') {
            push_field(&mut row, &mut field, &mut cells)?;
            at_start = true;
            if ch != '\t' {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                rows.push(std::mem::take(&mut row));
                if rows.len() > plancraft_core::limits::MAX_TASKS {
                    return Err(crate::tr("Clipboard data is too large").into());
                }
                ended_row = true;
            }
        } else {
            field.push(ch);
            at_start = false;
        }
    }
    if quoted {
        return Err(crate::tr("Clipboard text has an unclosed quoted cell").into());
    }
    // A terminal CRLF is a record terminator, not an additional empty task.
    if !ended_row || !row.is_empty() {
        push_field(&mut row, &mut field, &mut cells)?;
        rows.push(row);
    }
    if rows.len() > plancraft_core::limits::MAX_TASKS {
        return Err(crate::tr("Clipboard data is too large").into());
    }
    Ok(rows)
}

fn push_field(row: &mut Vec<String>, field: &mut String, cells: &mut usize) -> Result<(), String> {
    if row.len() >= MAX_COLS || *cells >= MAX_CELLS || field.chars().count() > plancraft_core::limits::MAX_NOTES_CHARS {
        return Err(crate::tr("Clipboard data is too large").into());
    }
    *cells = cells.saturating_add(1);
    row.push(std::mem::take(field));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_text_round_trip_preserves_empty_unicode_and_quoted_cells() {
        let rows = vec![
            vec!["日本語".into(), "a\tb".into(), "line1\nline2\r\n\"quote\"".into(), String::new()],
            vec![String::new(), "tail".into()],
        ];
        let text = encode(&rows);
        assert_eq!(decode(&text).unwrap(), rows);
        assert_eq!(decode(&(text + "\r\n")).unwrap(), rows);
        assert_eq!(decode("a\t\r\nb\t\n").unwrap(), vec![vec!["a", ""], vec!["b", ""]]);
        assert_eq!(decode("").unwrap(), vec![vec![""]]);
    }

    #[test]
    fn malformed_and_oversized_clipboards_are_rejected_without_panics() {
        assert!(decode("\"unclosed").is_err());
        assert!(decode(&"\t".repeat(MAX_COLS)).is_err());
        assert!(decode(&"\n".repeat(plancraft_core::limits::MAX_TASKS + 1)).is_err());
        assert!(decode(&"x".repeat(MAX_BYTES + 1)).is_err());
        for text in ["\"", "\"\"", "\"\"\"", "\r", "\r\n", "\t", "a\"b", "\0\t\n", "💚\t日本語"] {
            let _ = decode(text);
        }
    }
}
