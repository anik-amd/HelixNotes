use super::operations::{
    ensure_vault_structure, import_note as import_from_root, ExternalNoteReference,
};

fn import_note(
    vault: &str,
    source: &str,
    notebook: Option<&str>,
    raw: &str,
    references: Vec<ExternalNoteReference>,
    authorize: impl Fn(&Path) -> Result<(), String>,
) -> Result<crate::types::NoteContent, String> {
    import_from_root(
        vault,
        source,
        Path::new(source).parent().unwrap(),
        notebook,
        raw,
        references,
        authorize,
    )
}
use std::fs;
use std::path::{Path, PathBuf};

fn reference(raw: &str, original: &str, href: &str, html: bool) -> ExternalNoteReference {
    let from = raw.find(original).unwrap();
    ExternalNoteReference {
        from,
        to: from + original.len(),
        href: href.into(),
        html,
        pdf: false,
    }
}

fn fixture(raw: &str) -> (tempfile::TempDir, String, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let vault = root.path().join("vault").to_string_lossy().into_owned();
    ensure_vault_structure(&vault).unwrap();
    let external = root.path().join("external");
    fs::create_dir_all(external.join("images")).unwrap();
    let source = external.join("External.md");
    fs::write(&source, raw).unwrap();
    (root, vault, source)
}

fn attachments(vault: &str) -> Vec<PathBuf> {
    fs::read_dir(Path::new(vault).join(".helixnotes/attachments"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect()
}

#[test]
fn imports_relative_files_once_preserving_originals_frontmatter_and_literal_content() {
    let raw = "---\r\ntitle: Portable\r\ncustom: 雪 📝\r\n---\r\n![One](images/café%20雪.png)\r\n![Two](images/./café%20雪.png)\r\n<a href=\"images/report.pdf#page=2&amp;mode=fit\">PDF</a>\r\n\r\n~~~md\r\n![Example](missing.png)\r\n~~~\r\n";
    for notebook in [None, Some("Projects/Nested")] {
        let (_root, vault, source) = fixture(raw);
        let parent = source.parent().unwrap();
        fs::write(parent.join("images/café 雪.png"), b"image bytes").unwrap();
        fs::write(parent.join("images/report.pdf"), b"pdf bytes").unwrap();
        fs::create_dir_all(Path::new(&vault).join("Projects/Nested")).unwrap();
        let references = vec![
            reference(raw, "images/café%20雪.png", "images/café%20雪.png", false),
            reference(
                raw,
                "images/./café%20雪.png",
                "images/./café%20雪.png",
                false,
            ),
            reference(
                raw,
                "images/report.pdf#page=2&amp;mode=fit",
                "images/report.pdf#page=2&mode=fit",
                true,
            ),
        ];
        let note = import_note(
            &vault,
            source.to_str().unwrap(),
            notebook,
            raw,
            references,
            |path| {
                if path.starts_with(parent) {
                    Ok(())
                } else {
                    Err("Not selected".into())
                }
            },
        )
        .unwrap();
        assert!(note
            .raw
            .starts_with("---\r\ntitle: Portable\r\ncustom: 雪 📝\r\n---\r\n"));
        assert!(note.raw.contains("![Example](missing.png)"));
        assert!(note.raw.contains("#page=2&amp;mode=fit"));
        assert_eq!(fs::read_to_string(&source).unwrap(), raw);
        assert_eq!(fs::read_to_string(&note.path).unwrap(), note.raw);
        let files = attachments(&vault);
        assert_eq!(
            files.len(),
            2,
            "aliases of one file share one portable copy"
        );
        for file in files {
            let filename = urlencoding::encode(file.file_name().unwrap().to_str().unwrap());
            let relative = format!(".helixnotes/attachments/{filename}");
            assert!(note.raw.contains(&relative));
            assert!(Path::new(&vault)
                .join(urlencoding::decode(&relative).unwrap().as_ref())
                .is_file());
        }
        assert_eq!(
            fs::read(parent.join("images/café 雪.png")).unwrap(),
            b"image bytes"
        );
        assert_eq!(
            fs::read(parent.join("images/report.pdf")).unwrap(),
            b"pdf bytes"
        );
    }
}

#[test]
fn imported_pdf_embeds_keep_the_vault_root_path_convention_in_nested_notebooks() {
    let raw = "<div data-pdf-src=\"images/report.pdf\" data-pdf-name=\"Report\"></div>";
    let (_root, vault, source) = fixture(raw);
    fs::write(source.parent().unwrap().join("images/report.pdf"), b"pdf").unwrap();
    fs::create_dir_all(Path::new(&vault).join("Projects/Nested")).unwrap();
    let mut pdf = reference(raw, "images/report.pdf", "images/report.pdf", true);
    pdf.pdf = true;
    let note = import_note(
        &vault,
        source.to_str().unwrap(),
        Some("Projects/Nested"),
        raw,
        vec![pdf],
        |_| Ok(()),
    )
    .unwrap();
    assert!(note.raw.contains("data-pdf-src=\".helixnotes/attachments/"));
    assert!(note.raw.contains("data-pdf-name=\"Report\""));
    assert_eq!(attachments(&vault).len(), 1);
}

#[test]
fn imported_document_links_remain_managed_and_stable_after_note_and_notebook_moves() {
    let raw = "[Report](images/report.pdf#page=2)";
    let (_root, vault, source) = fixture(raw);
    fs::write(source.parent().unwrap().join("images/report.pdf"), b"pdf").unwrap();
    fs::create_dir_all(Path::new(&vault).join("Projects/Nested")).unwrap();
    fs::create_dir_all(Path::new(&vault).join("Archive")).unwrap();
    let note = import_note(
        &vault,
        source.to_str().unwrap(),
        Some("Projects/Nested"),
        raw,
        vec![reference(
            raw,
            "images/report.pdf#page=2",
            "images/report.pdf#page=2",
            false,
        )],
        |_| Ok(()),
    )
    .unwrap();
    assert!(
        note.raw.starts_with("[Report](.helixnotes/attachments/"),
        "managed links must not depend on notebook depth"
    );
    let href = note
        .raw
        .strip_prefix("[Report](")
        .unwrap()
        .strip_suffix(')')
        .unwrap();
    let attachment = Path::new(&vault).join(
        urlencoding::decode(href.split('#').next().unwrap())
            .unwrap()
            .as_ref(),
    );
    assert_eq!(fs::read(attachment).unwrap(), b"pdf");
    let moved = super::operations::move_note(&vault, &note.path, &vault)
        .unwrap()
        .0;
    assert_eq!(fs::read_to_string(moved).unwrap(), note.raw);
    // Imported managed paths remain stable when their containing notebook is moved too.
    let second = import_note(
        &vault,
        source.to_str().unwrap(),
        Some("Projects/Nested"),
        raw,
        vec![reference(
            raw,
            "images/report.pdf#page=2",
            "images/report.pdf#page=2",
            false,
        )],
        |_| Ok(()),
    )
    .unwrap();
    super::operations::move_notebook(
        &vault,
        Path::new(&vault).join("Projects").to_str().unwrap(),
        Path::new(&vault).join("Archive").to_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(Path::new(&vault).join("Archive/Projects/Nested/External.md")).unwrap(),
        second.raw
    );
}

#[test]
fn import_does_not_replace_existing_note_or_same_named_attachment() {
    let raw = "![Image](images/photo.png)";
    let (_root, vault, source) = fixture(raw);
    fs::write(
        source.parent().unwrap().join("images/photo.png"),
        b"new image",
    )
    .unwrap();
    fs::write(Path::new(&vault).join("External.md"), b"keep note").unwrap();
    fs::write(
        Path::new(&vault).join(".helixnotes/attachments/photo.png"),
        b"keep image",
    )
    .unwrap();
    let note = import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        vec![reference(
            raw,
            "images/photo.png",
            "images/photo.png",
            false,
        )],
        |_| Ok(()),
    )
    .unwrap();
    assert!(note.path.ends_with("External (2).md"));
    assert_eq!(
        fs::read(Path::new(&vault).join("External.md")).unwrap(),
        b"keep note"
    );
    assert_eq!(
        fs::read(Path::new(&vault).join(".helixnotes/attachments/photo.png")).unwrap(),
        b"keep image"
    );
    assert_eq!(attachments(&vault).len(), 2);
}

#[test]
fn stale_sources_missing_files_and_denied_attachments_publish_nothing() {
    let raw = "![Image](images/photo.png)";
    let (_root, vault, source) = fixture(raw);
    let plan = || {
        vec![reference(
            raw,
            "images/photo.png",
            "images/photo.png",
            false,
        )]
    };
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        "stale",
        plan(),
        |_| Ok(())
    )
    .is_err());
    assert!(
        import_note(&vault, source.to_str().unwrap(), None, raw, plan(), |_| Ok(
            ()
        ))
        .is_err()
    );
    fs::write(source.parent().unwrap().join("images/photo.png"), b"image").unwrap();
    let denied = import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        plan(),
        |path| {
            if path == source {
                Ok(())
            } else {
                Err("Attachment not selected".into())
            }
        },
    );
    assert!(denied.unwrap_err().contains("not selected"));
    assert!(attachments(&vault).is_empty());
    assert!(!Path::new(&vault).join("External.md").exists());
    assert_eq!(fs::read_to_string(&source).unwrap(), raw);
}

#[test]
fn invalid_destinations_ranges_and_absolute_references_are_rejected() {
    let raw = "雪 ![Image](images/photo.png)";
    let (_root, vault, source) = fixture(raw);
    fs::write(source.parent().unwrap().join("images/photo.png"), b"image").unwrap();
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        Some("../escape"),
        raw,
        vec![],
        |_| Ok(())
    )
    .is_err());
    for href in [
        "/private.png",
        "%2Fprivate.png",
        "C:/private.png",
        "https://example.com/file.png",
        "Related.md",
    ] {
        assert!(import_note(
            &vault,
            source.to_str().unwrap(),
            None,
            raw,
            vec![reference(raw, "images/photo.png", href, false)],
            |_| Ok(())
        )
        .is_err());
    }
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        vec![ExternalNoteReference {
            from: 1,
            to: 3,
            href: "images/photo.png".into(),
            html: false,
            pdf: false,
        }],
        |_| Ok(())
    )
    .is_err());
    let plan = vec![
        reference(raw, "images/photo.png", "images/photo.png", false),
        reference(raw, "images/photo.png", "images/photo.png", false),
    ];
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        plan,
        |_| Ok(())
    )
    .is_err());
    assert!(attachments(&vault).is_empty());
}

#[test]
fn failed_note_publication_removes_only_its_new_attachments() {
    let raw = "![Image](images/photo.png)";
    let (_root, vault, source) = fixture(raw);
    fs::write(source.parent().unwrap().join("images/photo.png"), b"image").unwrap();
    fs::write(Path::new(&vault).join("External.md"), b"keep").unwrap();
    for number in 2..=100 {
        fs::write(
            Path::new(&vault).join(format!("External ({number}).md")),
            b"keep",
        )
        .unwrap();
    }
    let original_asset = Path::new(&vault).join(".helixnotes/attachments/original.png");
    fs::write(&original_asset, b"keep attachment").unwrap();
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        vec![reference(
            raw,
            "images/photo.png",
            "images/photo.png",
            false
        )],
        |_| Ok(())
    )
    .is_err());
    assert_eq!(attachments(&vault), vec![original_asset]);
    assert_eq!(
        fs::read(Path::new(&vault).join("External.md")).unwrap(),
        b"keep"
    );
    assert_eq!(fs::read_to_string(&source).unwrap(), raw);
    assert!(!fs::read_dir(&vault).unwrap().any(|entry| entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".helixnotes-import-")));
}

#[test]
fn note_content_cannot_import_parent_files_or_hidden_secrets_from_a_broad_scope() {
    let raw = "[File](asset.bin)";
    let (root, vault, source) = fixture(raw);
    let parent = source.parent().unwrap();
    for name in [
        ".ssh/key.bin",
        ".helixnotes/history/private.bin",
        ".helixnotes/attachments/photo.bin",
    ] {
        let file = parent.join(name);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, b"private fixture").unwrap();
    }
    fs::write(root.path().join("private.bin"), b"private outside fixture").unwrap();
    for href in [
        "../private.bin",
        ".ssh/key.bin",
        ".helixnotes/history/private.bin",
    ] {
        assert!(import_note(
            &vault,
            source.to_str().unwrap(),
            None,
            raw,
            vec![reference(raw, "asset.bin", href, false)],
            |_| Ok(())
        )
        .is_err());
        assert!(attachments(&vault).is_empty());
        assert!(!Path::new(&vault).join("External.md").exists());
    }
    let note = import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        vec![reference(
            raw,
            "asset.bin",
            ".helixnotes/attachments/photo.bin",
            false,
        )],
        |_| Ok(()),
    )
    .unwrap();
    assert!(note.raw.contains(".helixnotes/attachments/"));
    assert_eq!(attachments(&vault).len(), 1);
}

#[cfg(unix)]
#[test]
fn import_rejects_attachment_and_notebook_destinations_that_follow_symlinks() {
    let raw = "![Image](images/photo.png)";
    let (root, vault, source) = fixture(raw);
    fs::write(source.parent().unwrap().join("images/photo.png"), b"image").unwrap();
    let outside = root.path().join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, Path::new(&vault).join("Notebook")).unwrap();
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        Some("Notebook"),
        raw,
        vec![],
        |_| Ok(())
    )
    .is_err());
    let directory = Path::new(&vault).join(".helixnotes/attachments");
    fs::remove_dir(&directory).unwrap();
    std::os::unix::fs::symlink(&outside, &directory).unwrap();
    assert!(import_note(
        &vault,
        source.to_str().unwrap(),
        None,
        raw,
        vec![reference(
            raw,
            "images/photo.png",
            "images/photo.png",
            false
        )],
        |_| Ok(())
    )
    .is_err());
    assert!(fs::read_dir(outside).unwrap().next().is_none());
}
