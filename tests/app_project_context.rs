use lazydb::{app::App, project::ProjectContext};
use tempfile::TempDir;

#[test]
fn app_retains_the_startup_project_context() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("project");
    std::fs::create_dir_all(&root).unwrap();
    let project = ProjectContext::resolve_from(&root).unwrap();

    let app = App::with_startup_project(
        Vec::new(),
        Default::default(),
        Default::default(),
        project.clone(),
    );

    assert_eq!(app.project, project);
}
