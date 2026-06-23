//! [`World`] 実装。`typst-kit` の [`FileStore`]/[`SystemFiles`]（プロジェクト
//! ファイル + パッケージ解決）と [`FontStore`]（埋め込み + システムフォント）の
//! 上に薄く乗せる。

use std::path::Path;

use typst::World;
use typst::diag::FileResult;
use typst::foundations::{Bytes, Datetime, Duration};
use typst::syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot};
use typst::text::{Font, FontBook};
use typst::utils::LazyHash;
use typst::{Library, LibraryExt};
use typst_kit::downloader::SystemDownloader;
use typst_kit::files::{FileStore, FsRoot, SystemFiles};
use typst_kit::fonts::{self, FontStore};
use typst_kit::packages::SystemPackages;

use crate::EngineError;

/// Paladocs の [`World`]。1 つのプロジェクトルートと entrypoint（`root.typ`）を
/// 持ち、相対 import とパッケージ（`@preview/...`）を解決する。
pub(crate) struct PaladocsWorld {
    library: LazyHash<Library>,
    fonts: FontStore,
    files: FileStore<SystemFiles>,
    main: FileId,
}

impl PaladocsWorld {
    /// `root`（entrypoint の `.typ` ファイル）からワールドを構築する。
    ///
    /// プロジェクトルートは `root` の親ディレクトリ。フォントは埋め込み
    /// （`typst-assets`）+ システムフォントを探索する。パッケージは Typst
    /// Universe（`@preview`）から取得・キャッシュする。
    pub(crate) fn new(root: &Path) -> Result<Self, EngineError> {
        let root = root
            .canonicalize()
            .map_err(|e| EngineError::Io(format!("{}: {e}", root.display())))?;
        let parent = root.parent().ok_or_else(|| {
            EngineError::Io(format!("{}: has no parent directory", root.display()))
        })?;
        let file_name = root
            .file_name()
            .ok_or_else(|| EngineError::Io(format!("{}: not a file", root.display())))?
            .to_string_lossy()
            .into_owned();

        let vpath = VirtualPath::new(&file_name)
            .map_err(|e| EngineError::Io(format!("invalid entrypoint path: {e}")))?;
        let main = RootedPath::new(VirtualRoot::Project, vpath).intern();

        let project = FsRoot::new(parent.to_path_buf());
        let downloader = SystemDownloader::new(concat!("paladocs/", env!("CARGO_PKG_VERSION")));
        let packages = SystemPackages::new(downloader);
        let files = FileStore::new(SystemFiles::new(project, packages));

        let mut fonts = FontStore::new();
        fonts.extend(fonts::embedded());
        fonts.extend(fonts::system());

        Ok(Self {
            library: LazyHash::new(Library::default()),
            fonts,
            files,
            main,
        })
    }

    /// reload 用にファイルキャッシュを stale 化する。次のアクセスでローダ経由で
    /// 再読込される（[`FileStore::reset`]）。
    pub(crate) fn reset_files(&mut self) {
        self.files.reset();
    }
}

impl World for PaladocsWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        self.fonts.book()
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        self.files.source(id)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files.file(id)
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.font(index)
    }

    /// 再現可能な PDF を優先し、日付は常に `None` を返す（現在時刻に依存しない）。
    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}
