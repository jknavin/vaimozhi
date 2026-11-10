//! TXT and DOCX export. (PDF export uses the system print dialog from the UI so Tamil text is
//! shaped correctly by the browser engine; see documentation/ARCHITECTURE.md.)

use std::io::Write;
use std::path::Path;

pub fn export_txt(path: &Path, title: &str, text: &str) -> Result<(), String> {
    let mut body = String::from('\u{feff}'); // BOM so older Windows apps detect UTF-8
    if !title.trim().is_empty() {
        body.push_str(title.trim());
        body.push_str("\r\n\r\n");
    }
    body.push_str(&text.replace('\n', "\r\n"));
    std::fs::write(path, body).map_err(|e| format!("cannot write file: {e}"))
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn paragraph(text: &str, bold: bool) -> String {
    let b = if bold { "<w:b/><w:bCs/>" } else { "" };
    format!(
        "<w:p><w:r><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:cs=\"Nirmala UI\"/>{b}</w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
        xml_escape(text)
    )
}

pub fn docx_document_xml(title: &str, text: &str) -> String {
    let mut body = String::new();
    if !title.trim().is_empty() {
        body.push_str(&paragraph(title.trim(), true));
    }
    for line in text.split('\n') {
        body.push_str(&paragraph(line, false));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}</w:body></w:document>"
    )
}

pub fn export_docx(path: &Path, title: &str, text: &str) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| format!("cannot create file: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default();
    let err = |e: zip::result::ZipError| e.to_string();
    let io = |e: std::io::Error| e.to_string();

    zip.start_file("[Content_Types].xml", opts).map_err(err)?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
    )
    .map_err(io)?;

    zip.start_file("_rels/.rels", opts).map_err(err)?;
    zip.write_all(
        br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
    )
    .map_err(io)?;

    zip.start_file("word/document.xml", opts).map_err(err)?;
    zip.write_all(docx_document_xml(title, text).as_bytes()).map_err(io)?;
    zip.finish().map_err(err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_is_escaped_and_lines_become_paragraphs() {
        let xml = docx_document_xml("A & B", "line <1>\nவணக்கம்");
        assert!(xml.contains("A &amp; B"));
        assert!(xml.contains("line &lt;1&gt;"));
        assert!(xml.contains("வணக்கம்"));
        assert_eq!(xml.matches("<w:p>").count(), 3);
    }

    #[test]
    fn docx_is_a_valid_zip_with_document_part() {
        let path = std::env::temp_dir().join(format!("vaimozhi-export-{}.docx", std::process::id()));
        export_docx(&path, "Title", "hello").unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut s = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("word/document.xml").unwrap(), &mut s).unwrap();
        assert!(s.contains("hello"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn txt_has_bom_and_crlf() {
        let path = std::env::temp_dir().join(format!("vaimozhi-export-{}.txt", std::process::id()));
        export_txt(&path, "T", "a\nb").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
        assert!(String::from_utf8_lossy(&bytes).contains("a\r\nb"));
        let _ = std::fs::remove_file(&path);
    }
}
