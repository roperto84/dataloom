//! SQL helpers for the editor: formatting, parameter substitution, error-line
//! extraction and completion candidates.
//!
//! The formatter is deliberately conservative: it only reflows whitespace and
//! uppercases keywords outside strings/comments, so it can never change the
//! meaning of a statement. Anything it does not understand is left untouched.

const QUOTE: char = '\u{27}';
const BACKSLASH: char = '\u{5c}';

/// Keywords the formatter uppercases and the completion list offers.
pub const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "ON",
    "USING",
    "AS",
    "AND",
    "OR",
    "NOT",
    "NULL",
    "IS",
    "IN",
    "LIKE",
    "ILIKE",
    "BETWEEN",
    "EXISTS",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "VIEW",
    "INDEX",
    "ALTER",
    "DROP",
    "ADD",
    "COLUMN",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "UNIQUE",
    "DEFAULT",
    "CONSTRAINT",
    "DISTINCT",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "COALESCE",
    "CAST",
    "RETURNING",
    "UNION",
    "UNION ALL",
    "EXCEPT",
    "INTERSECT",
    "ASC",
    "DESC",
    "WITH",
    "RECURSIVE",
    "EXPLAIN",
    "ANALYZE",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
];

const BREAK_BEFORE: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP BY",
    "ORDER BY",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "RETURNING",
    "UNION",
    "UNION ALL",
    "EXCEPT",
    "INTERSECT",
    "JOIN",
    "LEFT JOIN",
    "RIGHT JOIN",
    "INNER JOIN",
    "FULL JOIN",
    "CROSS JOIN",
    "VALUES",
    "SET",
];

const CONTINUATION: &[&str] = &["AND", "OR"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TokenKind {
    Word,
    Number,
    Quoted,
    Comment,
    Punct,
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    kind: TokenKind,
}

/// Splits SQL into words, quoted values, comments and punctuation. A quote or
/// comment is kept verbatim so formatting cannot corrupt literal contents.
fn tokenize(sql: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = sql.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Comment,
            });
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let start = i;
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Comment,
            });
            continue;
        }
        if c == QUOTE || c == '"' || c == '`' {
            let quote = c;
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == quote {
                    if chars.get(i + 1) == Some(&quote) {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(Token {
                text: chars[start..i].iter().collect(),
                kind: TokenKind::Quoted,
            });
            continue;
        }
        if c.is_alphanumeric() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let kind = if text.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                TokenKind::Number
            } else {
                TokenKind::Word
            };
            tokens.push(Token { text, kind });
            continue;
        }
        // Two-character operators stay together so `>=`/`<>`/`::` read well.
        if i + 1 < chars.len() {
            let pair: String = chars[i..i + 2].iter().collect();
            if matches!(
                pair.as_str(),
                ">=" | "<=" | "<>" | "!=" | "::" | "||" | "->" | "=>"
            ) {
                tokens.push(Token {
                    text: pair,
                    kind: TokenKind::Punct,
                });
                i += 2;
                continue;
            }
        }
        tokens.push(Token {
            text: c.to_string(),
            kind: TokenKind::Punct,
        });
        i += 1;
    }
    tokens
}

/// Reflows a statement: major clauses start at column 0, `AND`/`OR`
/// continuations are indented, keywords are uppercased. String literals and
/// comments are copied verbatim, so formatting never changes the meaning.
pub fn format_sql(sql: &str) -> String {
    let raw = tokenize(sql);
    // Merge `GROUP BY`, `LEFT JOIN`… into one token so breaks land correctly.
    let mut merged: Vec<Token> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let token = &raw[i];
        if token.kind == TokenKind::Word {
            let upper = token.text.to_ascii_uppercase();
            if let Some(next) = raw.get(i + 1) {
                if next.kind == TokenKind::Word {
                    let pair = format!("{} {}", upper, next.text.to_ascii_uppercase());
                    if SQL_KEYWORDS.contains(&pair.as_str()) {
                        merged.push(Token {
                            text: pair,
                            kind: TokenKind::Word,
                        });
                        i += 2;
                        continue;
                    }
                }
            }
        }
        merged.push(token.clone());
        i += 1;
    }

    let mut out = String::new();
    let mut previous: Option<&Token> = None;
    for token in &merged {
        if token.kind == TokenKind::Comment {
            trim_trailing_space(&mut out);
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&token.text);
            out.push('\n');
            previous = None;
            continue;
        }

        let is_word = token.kind == TokenKind::Word;
        let upper = token.text.to_ascii_uppercase();
        let display = if is_word && SQL_KEYWORDS.contains(&upper.as_str()) {
            upper.clone()
        } else {
            token.text.clone()
        };
        let has_content = !out.trim_end().is_empty();

        if token.text == ";" {
            trim_trailing_space(&mut out);
            out.push(';');
            out.push('\n');
            previous = None;
            continue;
        }
        if token.text == "," {
            trim_trailing_space(&mut out);
            out.push_str(", ");
            previous = Some(token);
            continue;
        }
        if token.text == ")" {
            trim_trailing_space(&mut out);
            out.push(')');
            previous = Some(token);
            continue;
        }

        if is_word && BREAK_BEFORE.contains(&display.as_str()) && has_content {
            trim_trailing_space(&mut out);
            out.push('\n');
        } else if is_word && CONTINUATION.contains(&display.as_str()) && has_content {
            trim_trailing_space(&mut out);
            out.push_str("\n  ");
        } else if space_before(&out, previous, token) {
            out.push(' ');
        }
        out.push_str(&display);
        previous = Some(token);
    }

    let mut result = out
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    while result.ends_with('\n') {
        result.pop();
    }
    if !result.is_empty() && !result.ends_with(';') {
        result.push(';');
    }
    result
}

fn trim_trailing_space(out: &mut String) {
    while out.ends_with(' ') || out.ends_with('\t') {
        out.pop();
    }
}

/// Spacing between two tokens in the formatted output.
fn space_before(out: &str, previous: Option<&Token>, token: &Token) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if out.ends_with('\n') || out.ends_with(' ') || out.ends_with('(') {
        return false;
    }
    if token.text == "(" {
        let previous_upper = previous.text.to_ascii_uppercase();
        return matches!(
            previous_upper.as_str(),
            "IN" | "VALUES" | "EXISTS" | "AS" | "ON" | "USING" | "NOT"
        );
    }
    if token.text == "." || token.text == "::" {
        return false;
    }
    if previous.text == "." || previous.text == "::" {
        return false;
    }
    true
}

/// Extracts the 1-based line an engine reported, when present. PostgreSQL
/// prints `LINE 3:`, MySQL `at line 3`, SQLite has no line information.
pub fn error_line(error: &str) -> Option<usize> {
    let lower = error.to_ascii_lowercase();
    for marker in ["line ", "línea "] {
        let mut search = 0;
        while let Some(pos) = lower[search..].find(marker) {
            let start = search + pos + marker.len();
            let digits: String = lower[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(line) = digits.parse::<usize>() {
                if line > 0 {
                    return Some(line);
                }
            }
            search = start;
            if search >= lower.len() {
                break;
            }
        }
    }
    None
}

/// Named parameters in order of first appearance. `::` casts and `:=` are not
/// parameters; strings and comments are skipped.
pub fn parameters(sql: &str) -> Vec<String> {
    let tokens = tokenize(sql);
    let mut out: Vec<String> = Vec::new();
    for (i, token) in tokens.iter().enumerate() {
        if token.text != ":" || token.kind == TokenKind::Quoted {
            continue;
        }
        // `x::type` is a cast; only a colon followed by a word is a parameter.
        if i > 0 && tokens[i - 1].text == ":" {
            continue;
        }
        let Some(next) = tokens.get(i + 1) else {
            continue;
        };
        if next.kind == TokenKind::Word && next.text.chars().next().is_some_and(|c| c.is_alphabetic())
        {
            let name = next.text.clone();
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    out
}

/// Replaces `:name` with a literal, preserving the original whitespace and
/// every character outside the parameter tokens. Numbers, booleans and NULL
/// stay unquoted so `LIMIT :n` keeps working; everything else is quoted.
pub fn substitute_parameters(
    sql: &str,
    values: &[(String, String)],
    driver: crate::db::Driver,
) -> String {
    let chars: Vec<char> = sql.chars().collect();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    let mut previous = ' ';
    while i < chars.len() {
        let c = chars[i];
        // Copy quoted strings and comments verbatim.
        if c == QUOTE || c == '"' || c == '`' {
            let quote = c;
            out.push(c);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == quote {
                    if chars.get(i + 1) == Some(&quote) {
                        out.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            previous = quote;
            continue;
        }
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                out.push(chars[i]);
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            while i < chars.len() {
                out.push(chars[i]);
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    out.push('/');
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == ':' && previous != ':' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len()
                && (chars[end].is_alphanumeric() || chars[end] == '_')
            {
                end += 1;
            }
            let name: String = chars[start..end].iter().collect();
            if !name.is_empty()
                && chars[start].is_ascii_alphabetic()
                && chars.get(end) != Some(&':')
            {
                if let Some((_, value)) = values.iter().find(|(key, _)| key == &name) {
                    out.push_str(&sql_literal(value, driver));
                    i = end;
                    previous = '0';
                    continue;
                }
            }
        }
        out.push(c);
        previous = c;
        i += 1;
    }
    out
}

fn sql_literal(value: &str, driver: crate::db::Driver) -> String {
    let trimmed = value.trim();
    if trimmed.eq_ignore_ascii_case("null") {
        return "NULL".to_string();
    }
    if trimmed.eq_ignore_ascii_case("true") {
        return match driver {
            crate::db::Driver::Sqlite => "1".to_string(),
            _ => "TRUE".to_string(),
        };
    }
    if trimmed.eq_ignore_ascii_case("false") {
        return match driver {
            crate::db::Driver::Sqlite => "0".to_string(),
            _ => "FALSE".to_string(),
        };
    }
    if trimmed.parse::<i64>().is_ok() || trimmed.parse::<f64>().is_ok() {
        return trimmed.to_string();
    }
    let escaped = match driver {
        crate::db::Driver::MySql => trimmed.replace(BACKSLASH, "\\\\").replace(QUOTE, "''"),
        _ => trimmed.replace(QUOTE, "''"),
    };
    format!("'{escaped}'")
}

/// The word being typed at `cursor` (index in bytes) and its start index.
pub fn current_word(sql: &str, cursor: usize) -> (usize, String) {
    let cursor = cursor.min(sql.len());
    let before = &sql[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| !(c.is_alphanumeric() || *c == '_' || *c == '$'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    (start, before[start..].to_string())
}

/// Case-insensitive prefix suggestions, deduplicated, with columns and tables
/// ranked above keywords because they are more specific.
pub fn suggest(prefix: &str, sources: &[String], keywords: &[&str], limit: usize) -> Vec<String> {
    let prefix_lower = prefix.to_ascii_lowercase();
    if prefix_lower.chars().count() < 2 {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    for source in sources {
        if source.to_ascii_lowercase().starts_with(&prefix_lower)
            && !out.iter().any(|item| item.eq_ignore_ascii_case(source))
        {
            out.push(source.clone());
        }
    }
    for keyword in keywords {
        if keyword.to_ascii_lowercase().starts_with(&prefix_lower)
            && !out.iter().any(|item| item.eq_ignore_ascii_case(keyword))
        {
            out.push((*keyword).to_string());
        }
    }
    out.truncate(limit);
    out
}

/// Snippets offered by the editor's Templates menu. They are inserted at the
/// caret; `{}` marks where the caret should land afterwards.
pub fn template(name: &str) -> Option<&'static str> {
    match name {
        "SELECT" => Some(
            "SELECT {}\nFROM \nWHERE \nORDER BY \nLIMIT 100;",
        ),
        "JOIN" => Some(
            "SELECT a.*, b.*\nFROM  a\nJOIN  b ON b. = a.{}\nWHERE ",
        ),
        "INSERT" => Some(
            "INSERT INTO {} ()\nVALUES ();",
        ),
        "UPDATE" => Some(
            "UPDATE {}\nSET  = \nWHERE ;",
        ),
        "DELETE" => Some(
            "DELETE FROM {}\nWHERE ;",
        ),
        "CREATE TABLE" => Some(
            "CREATE TABLE {} (\n  id INTEGER PRIMARY KEY,\n  name TEXT NOT NULL\n);",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Driver;

    #[test]
    fn formatting_breaks_major_clauses_and_keeps_literals() {
        let sql = "select a,b from t where a='from where' and b=2 order by a desc";
        let formatted = format_sql(sql);
        assert!(formatted.starts_with("SELECT a, b\nFROM t"), "{formatted}");
        assert!(formatted.contains("WHERE a = 'from where'"), "{formatted}");
        assert!(formatted.contains("\n  AND b = 2"), "{formatted}");
        assert!(formatted.contains("\nORDER BY a DESC"), "{formatted}");
        assert!(formatted.ends_with(';'));
    }

    #[test]
    fn error_line_reads_engine_messages() {
        assert_eq!(error_line("ERROR: syntax error at or near \"x\"\nLINE 3: ..."), Some(3));
        assert_eq!(
            error_line("You have an error in your SQL syntax; check the manual at line 2"),
            Some(2)
        );
        assert_eq!(error_line("near \"SELEC\": syntax error"), None);
        assert_eq!(error_line("line 0"), None);
    }

    #[test]
    fn parameters_skip_casts_and_strings() {
        let sql = "SELECT ':not' || x::text FROM t WHERE a = :first AND b = :second";
        assert_eq!(parameters(sql), vec!["first", "second"]);
    }

    #[test]
    fn substitution_quotes_text_and_keeps_numbers() {
        let values = vec![
            ("name".to_string(), "O'Brien".to_string()),
            ("limit".to_string(), "10".to_string()),
            ("flag".to_string(), "true".to_string()),
        ];
        let sql = "SELECT * FROM t WHERE name = :name AND ok = :flag LIMIT :limit";
        let out = substitute_parameters(sql, &values, Driver::Sqlite);
        assert!(out.contains("name = 'O''Brien'"), "{out}");
        assert!(out.contains("ok = 1"), "{out}");
        assert!(out.contains("LIMIT 10"), "{out}");
        assert!(!out.contains(":limit"));
    }

    #[test]
    fn completion_matches_columns_before_keywords() {
        let sources = vec!["customers".to_string(), "customer_id".to_string()];
        let suggestions = suggest("cust", &sources, SQL_KEYWORDS, 8);
        assert_eq!(suggestions[0], "customers");
        assert_eq!(suggestions[1], "customer_id");
        assert!(suggest("c", &sources, SQL_KEYWORDS, 8).is_empty());
    }

    #[test]
    fn current_word_covers_unicode_boundaries() {
        let sql = "SELECT cliënt FROM t";
        let cursor = sql.find(" FROM").unwrap();
        assert_eq!(current_word(sql, cursor), (7, "cliënt".to_string()));
    }
}
