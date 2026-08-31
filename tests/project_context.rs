use std::fs;

use lazydb::project::ProjectContext;
use tempfile::TempDir;

#[test]
fn resolves_nearest_git_root_from_a_nested_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    let nested = root.join("src").join("feature");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir(root.join(".git")).unwrap();

    let context = ProjectContext::resolve_from(&nested).unwrap();

    assert_eq!(context.root, root.canonicalize().unwrap());
    assert_eq!(context.display_name, "repository");
}

#[test]
fn accepts_a_git_file_for_linked_worktrees() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("worktree");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join(".git"),
        "gitdir: /tmp/repository/.git/worktrees/worktree\n",
    )
    .unwrap();

    let context = ProjectContext::resolve_from(&root.join("src")).unwrap();

    assert_eq!(context.root, root.canonicalize().unwrap());
}

#[test]
fn falls_back_to_canonical_cwd_outside_git() {
    let temp = TempDir::new().unwrap();
    let nested = temp.path().join("standalone");
    fs::create_dir_all(&nested).unwrap();

    let context = ProjectContext::resolve_from(&nested).unwrap();

    assert_eq!(context.root, nested.canonicalize().unwrap());
    assert_eq!(context.display_name, "standalone");
}

#[cfg(unix)]
#[test]
fn canonicalizes_a_symlinked_start_directory() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    let link = temp.path().join("alias");
    fs::create_dir(root.join(".git")).unwrap_or_else(|_| {
        fs::create_dir_all(root.join(".git")).unwrap();
    });
    std::os::unix::fs::symlink(&root, &link).unwrap();

    let context = ProjectContext::resolve_from(&link).unwrap();

    assert_eq!(context.root, root.canonicalize().unwrap());
}
