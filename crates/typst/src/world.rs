//! [`World`] 実装。`typst-kit` の [`FileStore`]/[`SystemFiles`]（プロジェクト
//! ファイル + パッケージ解決）と [`FontStore`]（埋め込み + システムフォント）の
//! 上に薄く乗せる。

use std::path::Path;

use chrono::{Datelike, Duration as ChronoDuration, Local, Utc};
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
///
/// 上位（`cli`）が所有して [`compile_deck`](crate::compile_deck) に渡し、reload 時は
/// [`reset_files`](PaladocsWorld::reset_files) でファイルキャッシュを stale 化してから
/// 再コンパイルする。
pub struct PaladocsWorld {
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
    pub fn new(root: &Path) -> Result<Self, EngineError> {
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
    pub fn reset_files(&mut self) {
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

    /// 現在の日付を返す。`offset` が無ければシステムのローカルタイムゾーン、
    /// あれば UTC からその時間幅だけずらした日付を用いる（Typst の
    /// `datetime.today(offset)` 相当）。
    fn today(&self, offset: Option<Duration>) -> Option<Datetime> {
        let naive = match offset {
            None => Local::now().naive_local(),
            Some(o) => {
                let secs = o.seconds().round() as i64;
                (Utc::now() + ChronoDuration::seconds(secs)).naive_utc()
            }
        };
        Datetime::from_ymd(
            naive.year(),
            naive.month().try_into().ok()?,
            naive.day().try_into().ok()?,
        )
    }
}
