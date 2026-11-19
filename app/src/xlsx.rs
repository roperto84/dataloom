//! Dependency-free XLSX (Office Open XML) writer.
//!
//! The only public entry point is [`workbook`], which builds a minimal but
//! valid SpreadsheetML package in memory and returns the raw ZIP bytes. The
//! ZIP container is written by hand (STORE / method 0, CRC-32, central
//! directory, EOCD) so the export feature needs no new crates.
//!
//! Mapping rules:
//! * row 1 holds the `columns` headers, written as inline strings;
//! * data cells are inline strings too, except "numeric looking" values that
//!   fit Excel's 15 significant digits (`42`, `-3.5`), which become `<v>` numbers;
//! * values with leading zeros (`007`), more than 15 characters, or exponent
//!   forms (`1e5`) stay text so IDs and codes round-trip unchanged;
//! * `None` and empty strings produce empty cells;
//! * every cell carries a `r` reference, including AA.. for wide tables.
//!
//! Not supported on purpose: styles/formatting, formulas, dates, frozen panes,
//! hyperlinks, multiple sheets and shared strings. Text is XML-escaped and
//! invalid XML control characters are dropped.

use std::fmt::Write as _;

/// Builds an `.xlsx` workbook with a single sheet and returns the ZIP bytes.
///
/// `sheet_name` is sanitized (forbidden characters removed, trimmed, at most
/// 31 chars, falling back to `Sheet1`). `columns` becomes the header row and
/// every data row is aligned to the widest of `columns` and `rows`, so the
/// exported grid is rectangular even when rows have ragged lengths.
pub fn workbook(sheet_name: &str, columns: &[String], rows: &[Vec<Option<String>>]) -> Vec<u8> {
    let sheet = sanitize_sheet_name(sheet_name);
    let entries: Vec<(&str, Vec<u8>)> = vec![
        ("[Content_Types].xml", content_types_xml().into_bytes()),
        ("_rels/.rels", root_rels_xml().into_bytes()),
        ("xl/workbook.xml", workbook_xml(&sheet).into_bytes()),
        (
            "xl/_rels/workbook.xml.rels",
            workbook_rels_xml().into_bytes(),
        ),
        (
            "xl/worksheets/sheet1.xml",
            worksheet_xml(columns, rows).into_bytes(),
        ),
    ];
    zip_store(&entries)
}

// ---------------------------------------------------------------------------
// Package parts
// ---------------------------------------------------------------------------

const XML_DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";

fn content_types_xml() -> String {
    format!(
        "{XML_DECL}\n\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>\
<Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>\
</Types>"
    )
}

fn root_rels_xml() -> String {
    format!(
        "{XML_DECL}\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/>\
</Relationships>"
    )
}

fn workbook_xml(sheet_name: &str) -> String {
    format!(
        "{XML_DECL}\n\
<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">\
<sheets><sheet name=\"{}\" sheetId=\"1\" r:id=\"rId1\"/></sheets>\
</workbook>",
        xml_escape(sheet_name)
    )
}

fn workbook_rels_xml() -> String {
    format!(
        "{XML_DECL}\n\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>\
</Relationships>"
    )
}

fn worksheet_xml(columns: &[String], rows: &[Vec<Option<String>>]) -> String {
    let width = columns
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut xml = String::with_capacity(1024 + width * 64 + rows.len() * width * 24);
    xml.push_str(XML_DECL);
    xml.push_str(
        "\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">\
<sheetData>",
    );

    let mut row_number: u32 = 1;
    if !columns.is_empty() {
        xml.push_str(&format!("<row r=\"{row_number}\">"));
        for column in 0..width {
            let reference = format!("{}{}", column_letter(column), row_number);
            let text = columns.get(column).map(String::as_str).unwrap_or("");
            push_text_cell(&mut xml, &reference, text);
        }
        xml.push_str("</row>");
        row_number += 1;
    }

    for row in rows {
        if width == 0 {
            // No columns known at all: keep the row marker so row numbers stay
            // aligned with the caller's row order.
            xml.push_str(&format!("<row r=\"{row_number}\"/>"));
        } else {
            xml.push_str(&format!("<row r=\"{row_number}\">"));
            for column in 0..width {
                let reference = format!("{}{}", column_letter(column), row_number);
                match row.get(column).and_then(Option::as_deref) {
                    None | Some("") => push_empty_cell(&mut xml, &reference),
                    Some(value) => match numeric_value(value) {
                        Some(number) => push_number_cell(&mut xml, &reference, number),
                        None => push_text_cell(&mut xml, &reference, value),
                    },
                }
            }
            xml.push_str("</row>");
        }
        row_number += 1;
    }

    xml.push_str("</sheetData></worksheet>");
    xml
}

fn push_text_cell(xml: &mut String, reference: &str, text: &str) {
    let _ = write!(
        xml,
        "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
        xml_escape(text)
    );
}

fn push_number_cell(xml: &mut String, reference: &str, number: &str) {
    // `number` was validated by `numeric_value`, so it is plain ASCII digits,
    // an optional leading '-', and at most one '.'.
    let _ = write!(xml, "<c r=\"{reference}\" t=\"n\"><v>{number}</v></c>");
}

fn push_empty_cell(xml: &mut String, reference: &str) {
    let _ = write!(xml, "<c r=\"{reference}\"/>");
}

// ---------------------------------------------------------------------------
// Value classification
// ---------------------------------------------------------------------------

/// Returns `Some(value)` when the string can be written as a numeric cell
/// without losing information.
///
/// Rejected on purpose: leading zeros, exponents, more than 15 characters
/// (Excel's precision limit), whitespace, `+` signs, `.5`, `5.` and anything
/// with non-ASCII characters. Those all fall back to inline strings.
fn numeric_value(value: &str) -> Option<&str> {
    if value.is_empty() || value.len() > 15 {
        return None;
    }
    let bytes = value.as_bytes();
    let mut index = 0usize;
    if bytes[index] == b'-' {
        index += 1;
    }
    let int_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let int_len = index - int_start;
    if int_len == 0 {
        return None;
    }
    // Keep codes like "007" as text.
    if int_len > 1 && bytes[int_start] == b'0' {
        return None;
    }
    if index < bytes.len() {
        if bytes[index] != b'.' {
            return None;
        }
        index += 1;
        let frac_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == frac_start {
            return None;
        }
    }
    if index == bytes.len() {
        Some(value)
    } else {
        None
    }
}

/// Escapes text for XML character data or a double-quoted attribute value.
/// Control characters that XML 1.0 forbids are dropped.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\t' | '\n' | '\r' => out.push(ch),
            '\u{FFFE}' | '\u{FFFF}' => {}
            ch if (ch as u32) < 0x20 => {}
            ch => out.push(ch),
        }
    }
    out
}

/// Excel column name for a zero-based index: 0 -> A, 25 -> Z, 26 -> AA.
fn column_letter(mut index: usize) -> String {
    let mut letters = Vec::with_capacity(4);
    loop {
        letters.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.reverse();
    String::from_utf8(letters).unwrap_or_else(|_| String::from("A"))
}

/// Removes the characters Excel forbids in sheet names, trims, strips
/// surrounding apostrophes, truncates to 31 chars and falls back to `Sheet1`.
fn sanitize_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|ch| !matches!(ch, '[' | ']' | ':' | '*' | '?' | '/' | '\\'))
        .filter(|ch| !ch.is_control())
        .collect();
    let trimmed = cleaned.trim().trim_matches('\'').trim();
    let truncated: String = trimmed.chars().take(31).collect();
    if truncated.is_empty() {
        "Sheet1".to_string()
    } else {
        truncated
    }
}

// ---------------------------------------------------------------------------
// ZIP container (STORE only, written by hand)
// ---------------------------------------------------------------------------

const ZIP_LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const ZIP_CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const ZIP_EOCD_SIGNATURE: u32 = 0x0605_4b50;
/// Bit 11: file names are UTF-8.
const ZIP_FLAG_UTF8: u16 = 0x0800;
const ZIP_VERSION: u16 = 20; // 2.0, supports STORE
/// 1980-01-01 00:00 in MS-DOS format (the ZIP epoch).
const ZIP_DOS_TIME: u16 = 0;
const ZIP_DOS_DATE: u16 = 0x0021;

fn zip_store(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let payload: usize = entries
        .iter()
        .map(|(name, data)| name.len() + data.len() + 30 + 46)
        .sum();
    let mut out = Vec::with_capacity(payload + 22);
    let mut central = Vec::with_capacity(entries.len() * 64);

    for (name, data) in entries {
        let name = name.as_bytes();
        let data: &[u8] = data.as_slice();
        let crc = crc32(data);
        let size = data.len() as u32;
        let offset = out.len() as u32;
        let name_len = name.len() as u16;

        // Local file header.
        out.extend_from_slice(&ZIP_LOCAL_HEADER_SIGNATURE.to_le_bytes());
        out.extend_from_slice(&ZIP_VERSION.to_le_bytes());
        out.extend_from_slice(&ZIP_FLAG_UTF8.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // method 0 = STORE
        out.extend_from_slice(&ZIP_DOS_TIME.to_le_bytes());
        out.extend_from_slice(&ZIP_DOS_DATE.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes()); // compressed size
        out.extend_from_slice(&size.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        // Central directory file header.
        central.extend_from_slice(&ZIP_CENTRAL_HEADER_SIGNATURE.to_le_bytes());
        central.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version made by
        central.extend_from_slice(&ZIP_VERSION.to_le_bytes()); // version needed
        central.extend_from_slice(&ZIP_FLAG_UTF8.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // method 0 = STORE
        central.extend_from_slice(&ZIP_DOS_TIME.to_le_bytes());
        central.extend_from_slice(&ZIP_DOS_DATE.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        central.extend_from_slice(&0u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }

    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);

    // End of central directory record.
    let count = entries.len() as u16;
    out.extend_from_slice(&ZIP_EOCD_SIGNATURE.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with central directory
    out.extend_from_slice(&count.to_le_bytes()); // entries on this disk
    out.extend_from_slice(&count.to_le_bytes()); // total entries
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// Standard CRC-32 (IEEE 802.3, the one ZIP uses).
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc = (crc >> 8) ^ CRC32_TABLE[((crc ^ byte as u32) & 0xFF) as usize];
    }
    !crc
}

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut index = 0usize;
    while index < 256 {
        let mut crc = index as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
            bit += 1;
        }
        table[index] = crc;
        index += 1;
    }
    table
};

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY_NAMES: [&str; 5] = [
        "[Content_Types].xml",
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/worksheets/sheet1.xml",
    ];

    fn rows(values: &[&[Option<&str>]]) -> Vec<Vec<Option<String>>> {
        values
            .iter()
            .map(|row| row.iter().map(|value| value.map(str::to_string)).collect())
            .collect()
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// Parses the ZIP local headers and returns the stored bytes for `wanted`,
    /// verifying method, sizes and CRC on the way.
    fn stored_entry(zip: &[u8], wanted: &str) -> Option<Vec<u8>> {
        assert!(zip.starts_with(&ZIP_LOCAL_HEADER_SIGNATURE.to_le_bytes()));
        let mut position = 0usize;
        while position + 30 <= zip.len() {
            let signature = read_u32(zip, position);
            if signature == ZIP_CENTRAL_HEADER_SIGNATURE {
                break;
            }
            assert_eq!(signature, ZIP_LOCAL_HEADER_SIGNATURE, "bad local header");
            let flags = read_u16(zip, position + 6);
            let method = read_u16(zip, position + 8);
            let crc = read_u32(zip, position + 14);
            let compressed_size = read_u32(zip, position + 18) as usize;
            let uncompressed_size = read_u32(zip, position + 22) as usize;
            let name_len = read_u16(zip, position + 26) as usize;
            let extra_len = read_u16(zip, position + 28) as usize;
            let name_start = position + 30;
            let name = std::str::from_utf8(&zip[name_start..name_start + name_len]).unwrap();
            let data_start = name_start + name_len + extra_len;
            let data = &zip[data_start..data_start + compressed_size];

            assert_eq!(flags & ZIP_FLAG_UTF8, ZIP_FLAG_UTF8, "missing UTF-8 flag");
            assert_eq!(method, 0, "expected STORE method");
            assert_eq!(compressed_size, uncompressed_size);
            assert_eq!(crc32(data), crc, "CRC mismatch for {name}");

            if name == wanted {
                return Some(data.to_vec());
            }
            position = data_start + compressed_size;
        }
        None
    }

    fn read_u16(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
    }

    fn sheet_xml(columns: &[String], data_rows: &[Vec<Option<String>>]) -> String {
        let zip = workbook("Sheet1", columns, data_rows);
        let sheet = stored_entry(&zip, "xl/worksheets/sheet1.xml")
            .expect("worksheet part missing from the archive");
        String::from_utf8(sheet).expect("worksheet is not valid UTF-8")
    }

    #[test]
    fn zip_container_has_local_and_central_headers_with_all_parts() {
        let zip = workbook(
            "people",
            &strings(&["id", "name"]),
            &rows(&[&[Some("1"), Some("Ada")]]),
        );

        assert!(zip.starts_with(b"PK\x03\x04"));

        for name in ENTRY_NAMES {
            let entry = stored_entry(&zip, name);
            assert!(entry.is_some(), "missing entry {name}");
            assert!(!entry.unwrap().is_empty(), "empty entry {name}");
        }

        // EOCD is the last 22 bytes and must announce five entries.
        let eocd = zip.len() - 22;
        assert_eq!(read_u32(&zip, eocd), ZIP_EOCD_SIGNATURE);
        assert_eq!(read_u16(&zip, eocd + 8), 5); // entries on this disk
        assert_eq!(read_u16(&zip, eocd + 10), 5); // total entries
        let central_size = read_u32(&zip, eocd + 12) as usize;
        let central_offset = read_u32(&zip, eocd + 16) as usize;
        assert_eq!(central_offset + central_size, eocd);
        assert_eq!(read_u32(&zip, central_offset), ZIP_CENTRAL_HEADER_SIGNATURE);
    }

    #[test]
    fn headers_and_values_are_xml_escaped() {
        let xml = sheet_xml(
            &strings(&["a<b", "c&d"]),
            &rows(&[&[Some("x>y \"quoted\" 'single' & <tag>"), Some("plain")]]),
        );

        assert_eq!(xml.matches("<row r=").count(), 2);
        assert!(xml.contains("a&lt;b"), "header '<' not escaped: {xml}");
        assert!(xml.contains("c&amp;d"), "header '&' not escaped: {xml}");
        assert!(
            xml.contains("x&gt;y &quot;quoted&quot; &apos;single&apos; &amp; &lt;tag&gt;"),
            "cell value not escaped: {xml}"
        );
        assert!(!xml.contains("<is><t xml:space=\"preserve\">x>y"));
    }

    #[test]
    fn numeric_values_stay_numbers_but_ids_stay_text() {
        let xml = sheet_xml(
            &strings(&["count", "code", "long", "exp"]),
            &rows(&[&[
                Some("42"),
                Some("007"),
                Some("12345678901234567"),
                Some("1e5"),
            ]]),
        );

        assert!(xml.contains("<c r=\"A2\" t=\"n\"><v>42</v></c>"), "{xml}");
        assert!(
            xml.contains("<c r=\"B2\" t=\"inlineStr\"><is><t xml:space=\"preserve\">007</t>"),
            "leading zeros must stay text: {xml}"
        );
        assert!(
            xml.contains("12345678901234567</t></is>"),
            "16+ char values must stay text: {xml}"
        );
        assert!(
            xml.contains("<c r=\"D2\" t=\"inlineStr\">") && xml.contains(">1e5</t>"),
            "exponent values must stay text: {xml}"
        );

        // Negative decimals are numbers; empty/None cells stay empty.
        let xml = sheet_xml(
            &strings(&["a", "b", "c", "d"]),
            &rows(&[&[Some("-3.5"), Some(""), None, Some("0")]]),
        );
        assert!(xml.contains("<c r=\"A2\" t=\"n\"><v>-3.5</v></c>"), "{xml}");
        assert!(xml.contains("<c r=\"B2\"/>"), "{xml}");
        assert!(xml.contains("<c r=\"C2\"/>"), "{xml}");
        assert!(xml.contains("<c r=\"D2\" t=\"n\"><v>0</v></c>"), "{xml}");
    }

    #[test]
    fn columns_past_z_use_double_letters() {
        let columns: Vec<String> = (0..27).map(|index| format!("col{index}")).collect();
        let xml = sheet_xml(&columns, &[]);

        assert!(xml.contains("r=\"Z1\""), "missing Z column: {xml}");
        assert!(xml.contains("r=\"AA1\""), "missing AA column: {xml}");
        assert!(xml.contains(">col26</t>"), "wrong AA header: {xml}");
    }

    #[test]
    fn sheet_name_is_sanitized_and_truncated() {
        let sheet_of = |name: &str| {
            let zip = workbook(name, &[], &[]);
            String::from_utf8(stored_entry(&zip, "xl/workbook.xml").unwrap()).unwrap()
        };

        let xml = sheet_of("[bad]:*?/\\ name");
        assert!(xml.contains("name=\"bad name\""), "{xml}");

        let xml = sheet_of("abcdefghijklmnopqrstuvwxyz0123456789");
        assert!(xml.contains("name=\"abcdefghijklmnopqrstuvwxyz01234\""), "{xml}");

        let xml = sheet_of("   ");
        assert!(xml.contains("name=\"Sheet1\""), "{xml}");

        let xml = sheet_of("say \"hi\"");
        assert!(xml.contains("name=\"say &quot;hi&quot;\""), "{xml}");
    }

    #[test]
    fn crc32_matches_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn writes_sample_workbook_for_external_validation() {
        let columns = strings(&["id", "name", "score", "notes"]);
        let data = rows(&[
            &[
                Some("1"),
                Some("Alice"),
                Some("12.5"),
                Some("<ok & fine>"),
            ],
            &[Some("2"), Some("Bob"), Some("007"), Some("")],
            &[None, Some("Zoë"), Some("12345678901234567"), None],
        ]);
        let zip = workbook("users", &columns, &data);

        assert!(zip.len() > 500, "suspiciously small workbook");
        std::fs::write("/tmp/dataloom-xlsx-test.xlsx", &zip)
            .expect("could not write /tmp/dataloom-xlsx-test.xlsx for external validation");

        // Shape sanity check: 1 header row + 3 data rows, 4 columns.
        let xml = String::from_utf8(
            stored_entry(&zip, "xl/worksheets/sheet1.xml").expect("missing worksheet"),
        )
        .unwrap();
        assert_eq!(xml.matches("<row r=").count(), 4);
        assert!(xml.contains("<c r=\"A1\" t=\"inlineStr\">"));
        assert!(xml.contains(">Alice</t>"));
        assert!(xml.contains("<c r=\"A4\"/>"), "NULL id must be empty: {xml}");
        assert!(xml.contains("<c r=\"D3\"/>"), "empty string must be empty: {xml}");
    }
}
