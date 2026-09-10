//! H8, the analyzer toolchain: what is resolved and what is installed
//! (spec 021).
//!
//! Not verbs. These touch the operator's filesystem and the analyzers
//! the ingest preflight spawns, never the store, so they have no place
//! in a closed vocabulary about a graph. They are the shallowest hole
//! in the ledger for a reason: `yeomna-code` already owns resolution,
//! and this is the surface over it.
//!
//! `status` calls the same `resolve_and_probe` the ingest preflight
//! calls, so the two cannot drift on resolution order or on how a
//! version is asked for.

use std::path::Path;

use yeomna_code::lsp::{managed_tools_dir, resolve_and_probe};

/// The analyzers Yeomna spawns. C++ needs none: libclang runs in
/// process during analysis, and the tree-sitter grammars are compiled
/// in, so neither is a binary an operator installs.
const ANALYZERS: [&str; 2] = ["rust-analyzer", "gopls"];

/// `yeomna tools status`: what each analyzer resolves to and whether it
/// answers.
///
/// Reports absence rather than failing on it (EC-5). An appliance
/// without gopls is not broken, it is an appliance that will fall back
/// to the structural graph for Go, which is the degradation H3 built.
pub fn status() -> String {
    let workspace = std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf());
    let managed = managed_tools_dir();
    let mut rows: Vec<[String; 4]> = Vec::new();
    for analyzer in ANALYZERS {
        // No configured pin here: the config file has no analyzer keys
        // yet, so resolution runs managed-then-PATH, and `source` in the
        // output is what says which won.
        let s = resolve_and_probe(analyzer, None, &workspace);
        let (state, detail) = match &s.outcome {
            Ok(version) => ("ok", version.lines().next().unwrap_or("").to_string()),
            Err(e) => ("absent", e.lines().next().unwrap_or("").to_string()),
        };
        rows.push([
            analyzer.to_string(),
            format!(
                "{}{}",
                s.source,
                if s.configured { " (pinned)" } else { "" }
            ),
            state.to_string(),
            detail,
        ]);
    }

    let headers = ["analyzer", "source", "state", "detail"];
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.chars().count());
        }
    }
    let line = |cells: &[String]| {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = String::new();
    // Header on the same stream as the rows.
    out.push_str(&line(&headers.map(String::from)));
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  "),
    );
    out.push('\n');
    for row in &rows {
        out.push_str(&line(row));
        out.push('\n');
    }
    out.push_str(&format!(
        "\nmanaged tools directory: {}\n",
        managed.display()
    ));
    out
}

/// Why an install did not happen.
#[derive(Debug)]
pub enum InstallError {
    /// The name is not one of the analyzers this appliance spawns.
    ///
    /// Checked because the name becomes a path component under the managed
    /// directory, so an unchecked one could carry `..` or be absolute and
    /// place a binary anywhere the process can write. The check lives in
    /// [`install_in`] rather than only in the CLI, so the library is safe
    /// whatever calls it.
    UnknownAnalyzer(String),
    /// R24: fetching over the network is a charter question, not a
    /// convenience, and this command does not decide it.
    NoSource,
    NotAFile(String),
    Io(String),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::NoSource => write!(
                f,
                "tools install needs --from <path> naming a binary you already have.\n\
                 Fetching from upstream is not implemented on purpose: a byte arriving\n\
                 from the network into a sealed appliance is a charter section 5 question\n\
                 and not a convenience, and it waits for a ruling (R24)."
            ),
            InstallError::UnknownAnalyzer(name) => write!(
                f,
                "{name:?} is not an analyzer this appliance spawns. It knows {}",
                ANALYZERS.join(", ")
            ),
            InstallError::NotAFile(p) => write!(f, "{p} is not a file"),
            InstallError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// `yeomna tools install --from <path>`: place a binary the operator
/// already has where the resolver will find it.
///
/// The managed directory exists so a pinned analyzer resolves the same
/// way from every working directory, which is what a rustup shim does
/// not do. This copies rather than links, so the installed binary does
/// not move when the operator's copy does.
pub fn install(analyzer: &str, from: Option<&str>) -> Result<String, InstallError> {
    install_in(&managed_tools_dir(), analyzer, from)
}

/// [`install`] with an explicit managed directory, so a test can prove
/// the placement without mutating the environment of a binary whose
/// tests run in parallel. The same split `resolve_and_probe_in` uses,
/// and for the same reason.
pub fn install_in(dir: &Path, analyzer: &str, from: Option<&str>) -> Result<String, InstallError> {
    // The name becomes a path component below, so it is checked against the
    // closed list before anything touches the filesystem. Without this,
    // `../../..` or an absolute path would place a binary outside the
    // managed directory.
    if !ANALYZERS.contains(&analyzer) {
        return Err(InstallError::UnknownAnalyzer(analyzer.to_string()));
    }
    let Some(from) = from else {
        return Err(InstallError::NoSource);
    };
    let source = Path::new(from);
    if !source.is_file() {
        return Err(InstallError::NotAFile(from.to_string()));
    }
    std::fs::create_dir_all(dir).map_err(|e| InstallError::Io(e.to_string()))?;
    let target = dir.join(analyzer);
    std::fs::copy(source, &target).map_err(|e| InstallError::Io(e.to_string()))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| InstallError::Io(e.to_string()))?;
    Ok(format!(
        "installed {analyzer} to {}\nrun `yeomna tools status` to see it resolve\n",
        target.display()
    ))
}

/// The analyzers this command knows, for the usage text.
pub fn known() -> &'static [&'static str] {
    &ANALYZERS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_reports_every_analyzer_with_its_header_on_one_stream() {
        let out = status();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("analyzer"), "{out}");
        assert!(lines[1].starts_with("----"), "{out}");
        for analyzer in ANALYZERS {
            assert!(
                out.contains(analyzer),
                "{analyzer} is missing from the report: {out}"
            );
        }
        assert!(out.contains("managed tools directory:"), "{out}");
    }

    /// R24: no source means no install, and the message says why rather
    /// than reaching for the network.
    #[test]
    fn install_without_a_source_refuses_and_says_why() {
        let e = install("rust-analyzer", None).expect_err("must refuse");
        assert!(matches!(e, InstallError::NoSource));
        let said = e.to_string();
        assert!(said.contains("--from"), "{said}");
        assert!(said.contains("section 5"), "it names the reason: {said}");
    }

    #[test]
    fn install_places_the_binary_and_makes_it_executable() {
        let dir = tempfile::TempDir::new().unwrap();
        let managed = dir.path().join("tools");
        let binary = dir.path().join("pretend-analyzer");
        std::fs::write(&binary, b"#!/bin/sh\necho 1.0.0\n").unwrap();
        let out = install_in(&managed, "rust-analyzer", Some(&binary.to_string_lossy()))
            .expect("installs");

        assert!(out.contains("installed rust-analyzer"), "{out}");
        let placed = managed.join("rust-analyzer");
        assert!(placed.is_file(), "the binary is where the resolver looks");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&placed).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "and it is executable");
    }

    #[test]
    fn install_refuses_a_path_that_is_not_a_file() {
        let e = install_in(Path::new("/tmp"), "gopls", Some("/nonexistent/binary"))
            .expect_err("must refuse");
        assert!(matches!(e, InstallError::NotAFile(_)));
    }

    /// The analyzer name is a path component, so it cannot be the caller's
    /// to choose. Refused before the filesystem is touched, and refused in
    /// the library rather than only at the CLI.
    #[test]
    fn install_refuses_a_name_that_is_not_an_analyzer() {
        let dir = tempfile::TempDir::new().unwrap();
        let managed = dir.path().join("tools");
        let binary = dir.path().join("payload");
        std::fs::write(&binary, b"#!/bin/sh\ntrue\n").unwrap();
        let source = binary.to_string_lossy().to_string();

        for name in [
            "../../../tmp/escaped",
            "/tmp/absolute",
            "rust-analyzer/../evil",
            "clangd",
            "",
        ] {
            let e = install_in(&managed, name, Some(&source))
                .expect_err("a name that is not an analyzer must be refused");
            assert!(
                matches!(e, InstallError::UnknownAnalyzer(_)),
                "{name:?} gave {e:?}"
            );
            assert!(
                !managed.exists(),
                "{name:?} reached the filesystem before being refused"
            );
        }
        // And nothing landed anywhere the traversal aimed at.
        assert!(!Path::new("/tmp/escaped").exists());
        assert!(!Path::new("/tmp/absolute").exists());
    }
}
