//! The test loop: Jina v4 token embeddings (real, from the charter) through
//! late_chunk_embeddings, validated against Jina's own document vector.
use std::fs;
use yeomna_chunking::{LateChunkConfig, late_chunk_embeddings};

fn read_f32(path: &str) -> Vec<f32> {
    let bytes = fs::read(path).expect(path);
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb)
}

fn main() {
    let dir = std::env::args().nth(1).expect("fixture dir");
    let meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(format!("{dir}/meta.json")).unwrap()).unwrap();
    let seq = meta["seq_len"].as_u64().unwrap() as usize;
    let dim = meta["hidden_dim"].as_u64().unwrap() as usize;
    let prefix_chars = meta["prefix_chars"].as_u64().unwrap() as usize;

    let flat = read_f32(&format!("{dir}/hidden_states.f32"));
    assert_eq!(flat.len(), seq * dim, "fixture shape mismatch");
    let tokens: Vec<Vec<f32>> = flat.chunks_exact(dim).map(|c| c.to_vec()).collect();
    let single = read_f32(&format!("{dir}/single_vec.f32"));

    // 1. Whole-document pool through late.rs must reproduce Jina's own vector.
    let whole = late_chunk_embeddings(
        &tokens,
        &LateChunkConfig { chunk_size_tokens: seq, overlap_tokens: 0 },
    );
    let c = cosine(&whole.embeddings[0], &single);
    println!("whole-doc late.rs pool vs Jina single_vec cosine: {c:.6}");
    assert!(c > 0.9999, "late.rs pooling does not match Jina's");

    // 2. Real late chunking at the default 500/200.
    let result = late_chunk_embeddings(&tokens, &LateChunkConfig::default());
    println!("chunks: {}", result.embeddings.len());
    assert_eq!(result.boundaries.first().unwrap().0, 0);
    assert_eq!(result.boundaries.last().unwrap().1, seq);
    for (i, e) in result.embeddings.iter().enumerate() {
        let n: f32 = e.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-4, "chunk {i} not unit norm: {n}");
    }
    println!("all chunk embeddings unit-norm: true");

    // 3. Every chunk vector lives in the document's embedding space:
    //    similarity to the whole-doc vector should be high but not 1.
    let sims: Vec<f32> = result.embeddings.iter().map(|e| cosine(e, &single)).collect();
    let min = sims.iter().cloned().fold(f32::MAX, f32::min);
    let max = sims.iter().cloned().fold(f32::MIN, f32::max);
    println!("chunk-to-document cosine range: {min:.4} .. {max:.4}");

    // 4. Boundary metadata: token windows convert to byte spans via offsets.
    let offsets = meta["offsets_in_prefixed_text"].as_array().unwrap();
    let (ts, te) = result.boundaries[1];
    let byte_start = offsets[ts].as_array().unwrap()[0].as_u64().unwrap() as usize;
    let byte_end = offsets[te - 1].as_array().unwrap()[1].as_u64().unwrap() as usize;
    let doc = fs::read_to_string("/home/todd/git/Yeomna/README.md").unwrap();
    let rebased = byte_start.saturating_sub(prefix_chars)..byte_end.saturating_sub(prefix_chars);
    let snippet: String = doc[rebased.clone()].chars().take(60).collect();
    println!("chunk 1 tokens {ts}..{te} -> bytes {rebased:?}");
    println!("chunk 1 opens: {snippet:?}");

    println!("LOOP CLOSED");
}
