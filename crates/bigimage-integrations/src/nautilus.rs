//! Nautilus — integração via `nautilus-python` extension.
//!
//! Historicamente instalávamos também um "scripts tree" em
//! `~/.local/share/nautilus/scripts/BigIris/` como fallback para quem
//! não tinha `nautilus-python` — mas isso gerava **duas entradas
//! distintas** no menu de clique-direito ("Scripts ▸ BigIris" e
//! "Íris ▸"), confundindo o usuário. A partir do BigLinux atual o
//! `nautilus-python` é dependência básica; dropamos o fallback e
//! mantemos apenas a extension Python como ponto único de entrada.
//!
//! O `uninstall` ainda limpa a árvore antiga caso ela esteja presente
//! de instalações anteriores — garante que usuários que rodaram
//! `install-integrations --user` numa versão velha e agora correm a
//! nova ficam com o menu limpo.
//!
//! Caminho único: `~/.local/share/nautilus-python/extensions/bigiris-menu.py`.
//! Submenu "Íris ▸" aparece top-level no clique-direito.

use std::io;
use std::path::PathBuf;

use crate::scope::ScopePaths;

const STALE_TREE_ROOT: &str = "BigIris";
const PYTHON_EXTENSION_FILENAME: &str = "bigiris-menu.py";

/// The Python extension body, embedded at compile time from
/// `data/nautilus/bigiris-menu.py` at the repo root.
const PYTHON_EXTENSION: &str = include_str!("../../../data/nautilus/bigiris-menu.py");

/// Instala a extension Python e limpa resíduos de uma eventual árvore
/// scripts legada (quem rodou `install-integrations` em versões
/// anteriores ficou com os dois, gerando menus duplicados).
pub fn install(paths: &ScopePaths) -> io::Result<Vec<PathBuf>> {
    let mut written = Vec::new();

    // Garbage collect: scripts tree antiga fica limpa antes de instalar
    // para não aparecer no menu Scripts → BigIris.
    let stale_root = paths.nautilus_scripts().join(STALE_TREE_ROOT);
    if stale_root.exists() {
        let _ = std::fs::remove_dir_all(&stale_root);
    }

    // nautilus-python extension: único ponto de entrada oficial.
    let ext_dir = paths.nautilus_python_extensions();
    std::fs::create_dir_all(&ext_dir)?;
    let ext_file = ext_dir.join(PYTHON_EXTENSION_FILENAME);
    std::fs::write(&ext_file, PYTHON_EXTENSION)?;
    written.push(ext_file);

    Ok(written)
}

/// Remove a extension Python. Também limpa a scripts tree antiga caso
/// ela tenha sobrevivido de uma instalação anterior.
pub fn uninstall(paths: &ScopePaths) -> io::Result<Vec<PathBuf>> {
    let mut removed = Vec::new();

    let stale_root = paths.nautilus_scripts().join(STALE_TREE_ROOT);
    if stale_root.exists() {
        removed.extend(collect_files(&stale_root)?);
        std::fs::remove_dir_all(&stale_root)?;
    }

    let ext_file = paths.nautilus_python_extensions().join(PYTHON_EXTENSION_FILENAME);
    if ext_file.exists() {
        std::fs::remove_file(&ext_file)?;
        removed.push(ext_file);
    }

    Ok(removed)
}

fn collect_files(dir: &std::path::Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            out.extend(collect_files(&path)?);
        } else {
            out.push(path);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn install_writes_only_python_extension() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        let written = install(&paths).unwrap();
        // Um único arquivo: a extension Python. Scripts tree não é mais
        // instalada (antes eram ACTIONS.len() + 1 arquivos).
        assert_eq!(written.len(), 1);
        let ext = paths.nautilus_python_extensions().join(PYTHON_EXTENSION_FILENAME);
        assert!(ext.exists(), "extensão python não instalada");
    }

    #[test]
    fn install_cleans_stale_scripts_tree() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        // Simula um install antigo com scripts tree.
        let stale = paths.nautilus_scripts().join(STALE_TREE_ROOT);
        std::fs::create_dir_all(stale.join("Converter")).unwrap();
        std::fs::write(stale.join("Converter").join("PNG"), "#!/bin/sh\necho hi\n").unwrap();
        assert!(stale.exists());

        install(&paths).unwrap();
        assert!(!stale.exists(), "scripts tree antiga deveria ter sido limpa");
    }

    #[test]
    fn python_extension_has_expected_hooks() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        install(&paths).unwrap();
        let ext = paths.nautilus_python_extensions().join(PYTHON_EXTENSION_FILENAME);
        let body = std::fs::read_to_string(&ext).unwrap();
        assert!(body.contains("class BigIrisMenuProvider"));
        assert!(body.contains("Nautilus.MenuProvider"));
        assert!(body.contains("_TOP_LABEL = \"BigIris\""));
        assert!(body.contains("--dialog=convert"));
    }

    #[test]
    fn uninstall_removes_python_extension_and_legacy_tree() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        install(&paths).unwrap();
        // Adiciona uma legacy tree pra simular um user com dois installs.
        let stale = paths.nautilus_scripts().join(STALE_TREE_ROOT);
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("dummy"), b"x").unwrap();

        let removed = uninstall(&paths).unwrap();
        assert!(!removed.is_empty());
        assert!(!paths.nautilus_python_extensions().join(PYTHON_EXTENSION_FILENAME).exists());
        assert!(!stale.exists());
    }

    #[test]
    fn uninstall_on_empty_dir_is_noop() {
        let tmp = TempDir::new().unwrap();
        let paths = ScopePaths::rooted_at(tmp.path());
        let removed = uninstall(&paths).unwrap();
        assert!(removed.is_empty());
    }
}
