use super::*;
use miette::Diagnostic as _;

const MINIMAL: &str = r#"(local declare {:name :n :on :github.check_suite.completed :intent "i"
                :may [] :runs-per-hour 1})
(fn gather [{} event])
(fn policy [facts {}])
{: declare : gather : policy}"#;

fn named(name: &str) -> String {
    MINIMAL.replace(":name :n", &format!(":name :{name}"))
}

fn code(err: &Error) -> String {
    err.code().map(|c| c.to_string()).unwrap_or_default()
}

fn bundle(names: &[&str]) -> Bundle {
    Bundle {
        workflows: names
            .iter()
            .map(|name| Bundled {
                name: format!("{name}.fnl"),
                source: named(name),
            })
            .collect(),
    }
}

#[test]
fn a_directory_bundles_its_workflows_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    fs_err::write(dir.path().join("b.fnl"), named("b")).unwrap();
    fs_err::write(dir.path().join("a.fnl"), named("a")).unwrap();
    fs_err::write(dir.path().join("notes.md"), "not a workflow").unwrap();
    let read = Bundle::read_dir(dir.path()).unwrap();
    assert_eq!(read, bundle(&["a", "b"]));

    let loaded = read.load(&Catalog::shipped()).unwrap();
    let names: Vec<&str> = loaded
        .iter()
        .map(|w| w.declaration().name.as_str())
        .collect();
    assert_eq!(names, ["a", "b"]);

    let missing = Bundle::read_dir(&dir.path().join("missing")).unwrap_err();
    assert!(missing.to_string().contains("missing"), "{missing}");
}

#[test]
fn a_bundle_that_does_not_load_refuses_every_workflow_by_its_file_name() {
    let mut broken = bundle(&["a"]);
    broken.workflows.push(Bundled {
        name: "b.fnl".into(),
        source: "(oops".into(),
    });
    let err = broken.load(&Catalog::shipped()).unwrap_err();
    assert_eq!(code(&err), "arbitrary_almanac::parse");
    let source = err
        .source_code()
        .and_then(|src| src.read_span(&(0, 0).into(), 0, 0).ok());
    assert_eq!(
        source.and_then(|s| s.name().map(str::to_string)).as_deref(),
        Some("b.fnl")
    );
}

/// Writes `bytes` to a file and reads back its bundle.
fn read_back(bytes: &[u8]) -> Result<Option<Bundle>, Error> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("almanac");
    fs_err::write(&path, bytes).unwrap();
    Bundle::of_binary(&path)
}

#[test]
fn a_binary_carries_its_last_bundle_and_nothing_else() {
    let binary = b"\x7fELF and the rest of a binary".to_vec();
    let first = bundle(&["a"]).append_to(&binary);
    let second = bundle(&["b", "c"]).append_to(&first);
    let cases = [
        (binary.clone(), None),
        (b"short".to_vec(), None),
        (first.clone(), Some(bundle(&["a"]))),
        // A rebuild replaces the bundle rather than stacking another on it.
        (second.clone(), Some(bundle(&["b", "c"]))),
    ];
    for (bytes, expected) in cases {
        assert_eq!(read_back(&bytes).unwrap(), expected);
    }
    assert_eq!(second.len(), bundle(&["b", "c"]).append_to(&binary).len());
    assert_eq!(unbundled(&second), binary.as_slice());
}

#[test]
fn a_damaged_bundle_is_refused() {
    let bundled = bundle(&["a"]).append_to(b"binary");
    let mut garbled = bundled.clone();
    if let Some(byte) = garbled.get_mut(6) {
        *byte = b'#';
    }
    let mut overlong = b"x".to_vec();
    overlong.extend_from_slice(&u64::MAX.to_le_bytes());
    overlong.extend_from_slice(MAGIC);
    for bytes in [garbled, overlong.clone()] {
        let err = read_back(&bytes).unwrap_err();
        assert_eq!(code(&err), "arbitrary_almanac::bundle");
    }
    // Building over a damaged bundle keeps every byte rather than cutting into the binary.
    assert_eq!(unbundled(&overlong), overlong.as_slice());
}

#[test]
fn build_checks_the_workflows_before_writing_anything() {
    let dir = tempfile::tempdir().unwrap();
    let workflows = dir.path().join("workflows");
    fs_err::create_dir(&workflows).unwrap();
    fs_err::write(workflows.join("a.fnl"), named("a")).unwrap();
    let binary = dir.path().join("generic");
    fs_err::write(&binary, b"binary").unwrap();
    let out = dir.path().join("almanac");

    let built = build(&Catalog::shipped(), &workflows, &binary, &out).unwrap();
    assert_eq!(built, bundle(&["a"]));
    assert_eq!(Bundle::of_binary(&out).unwrap(), Some(bundle(&["a"])));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs_err::metadata(&out).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    fs_err::write(workflows.join("b.fnl"), "(oops").unwrap();
    let refused = dir.path().join("refused");
    assert!(build(&Catalog::shipped(), &workflows, &binary, &refused).is_err());
    assert!(!refused.exists());
}
