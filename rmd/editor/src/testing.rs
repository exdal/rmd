use base::{arena::StrArena, path::TreePath};
use std::{
    env, fs,
    ops::Deref,
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicUsize, Ordering},
};

use dmm::Prefab;

use crate::{
    environment::{BakeProgram, Environment},
    session::fixtures::examples,
};

pub(crate) const PASSWORD: &str = "hunter2";

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(name: &str) -> Self {
        let n = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = env::temp_dir().join(format!("rmd-{name}-{}-{n}", process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();

        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Deref for TempDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub(crate) fn prefab(path: &str) -> Prefab {
    Prefab::new(TreePath::parse(path))
}

pub(crate) fn compile_environment(prelude: &[(&'static str, &'static str)]) -> Environment {
    let root = examples();
    let compile = |baking| {
        let arena = StrArena::new();
        let files = preprocessor::prelude_files().into_iter().chain(
            prelude
                .iter()
                .map(|&(name, source)| preprocessor::PreludeFile::Embedded(name, source)),
        );
        let preprocessed = preprocessor::Preprocessor::new(&arena)
            .with_prelude(files)
            .with_baking(baking)
            .run(root.join("test.dm"))
            .expect("preprocess");
        assert!(preprocessed.is_ok(), "{:?}", preprocessed.errors);

        let ast = ast::parse(&preprocessed.tokens).expect("parse");
        let (tree, errors) = sema::analyze(&ast);
        assert!(errors.is_empty(), "{errors:?}");
        let (module, _) = ir::lower(&ast, &tree, baking);

        (tree, module)
    };
    let (editor_tree, _) = compile(false);
    let (bake_tree, module) = compile(true);
    let profile = vm::bake::profile_type(&bake_tree).expect("default profile");
    let mut environment = Environment::new(root.join("test.dme"), editor_tree);
    environment.bake_program = Some(BakeProgram {
        tree: bake_tree,
        module: codegen::generate(&module).expect("codegen"),
        profile,
        icon_states: Default::default(),
        uses_external_libraries: false,
    });

    environment
}
