//! Verify generated protobuf types compile and are accessible.
//!
//! The extraction test from the reference's `proto_types.rs`. The common,
//! embedding, and training sections dropped with their trimmed packages,
//! per spec 003 FR-P2.

use yeomna_proto::extraction::{ExtractRequest, ExtractResponse, ExtractorInfo, SourceType, Table};

#[test]
fn test_extraction_types() {
    let req = ExtractRequest {
        file_path: "/tmp/paper.pdf".to_string(),
        content: vec![],
        source_type: SourceType::Pdf.into(),
        extract_tables: true,
        extract_equations: true,
        extract_images: false,
        use_ocr: false,
    };
    assert_eq!(req.source_type, SourceType::Pdf as i32);
    assert!(req.extract_tables);

    let table = Table {
        content: "col1 | col2".to_string(),
        caption: "Table 1".to_string(),
        index: 0,
    };

    let resp = ExtractResponse {
        full_text: "Some extracted text".to_string(),
        tables: vec![table],
        equations: vec![],
        images: vec![],
        metadata: [("pages".to_string(), "10".to_string())].into(),
        source_type: SourceType::Pdf.into(),
    };
    assert_eq!(resp.tables.len(), 1);
    assert_eq!(resp.metadata.get("pages").unwrap(), "10");

    let info = ExtractorInfo {
        supported_extensions: vec![".pdf".to_string(), ".tex".to_string()],
        supported_types: vec![SourceType::Pdf.into(), SourceType::Latex.into()],
        features: vec!["ocr".to_string(), "tables".to_string()],
        gpu_available: true,
    };
    assert_eq!(info.supported_extensions.len(), 2);
}
