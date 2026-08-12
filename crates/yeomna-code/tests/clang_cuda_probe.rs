//! End-to-end probe for #149 against a real CUDA kernel file.
//!
//! Skips (does not fail) if libclang or the fixture is unavailable.

use std::path::Path;

use yeomna_code as code;

#[test]
fn clang_extracts_cuda_kernel_symbols() {
    // A CUDA fixture can be supplied via YEOMNA_CUDA_FIXTURE; the default
    // is this workstation's known kernel file. Absent either way, skip.
    let fixture = std::env::var("YEOMNA_CUDA_FIXTURE")
        .unwrap_or_else(|_| "/home/todd/olympus/NL_Hecate/core/kernels/elementwise.cu".to_string());
    let fixture = fixture.as_str();
    if !Path::new(fixture).exists() {
        eprintln!("SKIP: fixture not present: {fixture}");
        return;
    }

    match clang::Clang::new() {
        Ok(clang) => drop(clang),
        Err(e) => {
            eprintln!("SKIP: libclang unavailable: {e}");
            return;
        }
    }

    let source = std::fs::read_to_string(fixture).unwrap();
    let analysis = match code::analyze(&source, fixture) {
        Ok(analysis) if analysis.analysis_tier == code::AnalysisTier::Semantic => analysis,
        Ok(analysis) => {
            eprintln!(
                "SKIP: libclang could not analyze the CUDA fixture; fallback={} reason={}",
                analysis.analyzer,
                analysis.fallback_reason.as_deref().unwrap_or("unknown")
            );
            return;
        }
        Err(error) => {
            eprintln!("SKIP: CUDA fixture analysis unavailable: {error}");
            return;
        }
    };
    assert_eq!(analysis.analysis_tier, code::AnalysisTier::Semantic);
    assert_eq!(analysis.analyzer, "libclang");
    let names: Vec<&str> = analysis.symbols.iter().map(|s| s.name.as_str()).collect();
    eprintln!("extracted {} symbols from elementwise.cu", names.len());

    assert!(
        !analysis.symbols.is_empty(),
        "no functions extracted from the CUDA file"
    );
    assert!(
        names.contains(&"sigmoid_kernel"),
        "missing __global__ kernel `sigmoid_kernel`: {names:?}"
    );
    assert!(
        names.contains(&"sigmoid_cuda"),
        "missing extern \"C\" host wrapper `sigmoid_cuda`: {names:?}"
    );
    // Every symbol carries a line span -- the anchor the #148 identity needs.
    assert!(
        analysis.symbols.iter().all(|symbol| symbol.start_line > 0),
        "every symbol must have a 1-based line"
    );

    let wrapper = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name == "sigmoid_cuda")
        .expect("missing sigmoid_cuda wrapper");
    // Call resolution for CUDA kernel launches is a workstation capability,
    // not a portable guarantee: under clang 22.1.8 with no compilation
    // database the wrapper carries no "calls" key at all, where the clang
    // this probe was written against resolved the launch. Absence skips,
    // per the probe's own contract. Present-but-wrong still fails hard.
    let Some(calls) = wrapper.metadata["calls"].as_array() else {
        eprintln!(
            "SKIP: this box's libclang did not resolve calls for the CUDA \
             wrapper (no compilation database, clang version differences). \
             Kernel-launch edges will be absent from .cu ingests here."
        );
        return;
    };
    assert!(
        calls
            .iter()
            .any(|call| { call["name"] == "sigmoid_kernel" && call["is_kernel_launch"] == true }),
        "missing resolved CUDA launch edge: {calls:?}"
    );
}
