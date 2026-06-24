//! 不純シェル: engine・term・端末・スレッド・socket を結線し、[`Action`] ストリームを
//! [`crate::nav::step`] で回して [`RenderOp`] を実行する。
//!
//! ここで初めて実際の端末・スレッド・socket・プロセス境界に触れる。決定ロジックは
//! [`crate::nav`] に純粋に隔離してあるので、本モジュールは orchestration に徹する。

use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::thread;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use paladocs_core::{Deck, FrameId};
use paladocs_render::{CellGrid, Frame};
use paladocs_term::{CellSink, KittyBackend, Presenter, Viewport};
use paladocs_typst::{CompiledDeck, PaladocsWorld, RenderOpts, compile_deck, render_step};

use crate::cache::{FrameCache, Key};
use crate::cli::Mode;
use crate::diag;
use crate::error::CliError;
use crate::letterbox::{self, DEFAULT_BG};
use crate::mode::{OutputMode, detect_truecolor, resolve_mode, should_warn_truecolor};
use crate::nav::{self, Action, PresentState, RenderOp, step};
use crate::restore::{RawGuard, enter_sequence};
use crate::terminal::measure_viewport;

/// フレームキャッシュに保持する目安枚数（現 viewport の 1 フレーム相当 × これ）。
/// 現フレーム + 先読み候補（前後・隣接スライド先頭）を十分賄える小さな値。
const CACHE_FRAMES: usize = 12;

/// 現 viewport を基準にしたキャッシュ容量（バイト）。
fn cache_cap_bytes(viewport: Viewport) -> usize {
    viewport.pixel_size().byte_len().max(1) * CACHE_FRAMES
}

/// キー入力を [`Action`] へ写す純関数（端末に触れない）。
///
/// 割り当て:
/// - 進む: `→` / `Space` / `Enter` / `PageDown` / `j` / `l` / `n`
/// - 戻る: `←` / `Backspace` / `PageUp` / `k` / `h` / `p`
/// - 次スライド: `↓`、前スライド: `↑`
/// - 先頭へ: `Home`（= `Goto(0)`）
/// - 再読込: `r`、終了: `q` / `Esc` / `Ctrl-C`
///
/// 未割り当てキーは `None`。
pub fn map_key(code: KeyCode, modifiers: KeyModifiers) -> Option<Action> {
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }
    match code {
        KeyCode::Right
        | KeyCode::Char(' ')
        | KeyCode::Enter
        | KeyCode::PageDown
        | KeyCode::Char('j')
        | KeyCode::Char('l')
        | KeyCode::Char('n') => Some(Action::Advance),
        KeyCode::Left
        | KeyCode::Backspace
        | KeyCode::PageUp
        | KeyCode::Char('k')
        | KeyCode::Char('h')
        | KeyCode::Char('p') => Some(Action::Retreat),
        KeyCode::Down => Some(Action::NextSlide),
        KeyCode::Up => Some(Action::PrevSlide),
        KeyCode::Home => Some(Action::Goto(FrameId(0))),
        KeyCode::Char('r') => Some(Action::Reload),
        KeyCode::Char('q') | KeyCode::Esc => Some(Action::Quit),
        _ => None,
    }
}

/// 出口レンダラを抽象する不純な実行器。画像（[`Runner`]）と cell（[`CellRunner`]）は
/// 同一ナビ state machine の下で出口だけ差し替わる（ブリーフ不変条件 5）。
///
/// メインループ [`drive`] はこの trait だけを見て回す。`prefetch_one` の既定は「候補
/// 無し」で、アイドル先読みを持たないレンダラ（cell）はブロッキング待機へ移る。
trait Stage {
    /// 起動時の初期提示。
    fn present_initial(&mut self, sink: &mut dyn Write) -> io::Result<()>;
    /// 1 つの [`Action`] を処理する。`Ok(false)` で終了要求。
    fn handle(&mut self, sink: &mut dyn Write, action: Action) -> io::Result<bool>;
    /// アイドル時に先読み候補を 1 枚だけ温める。`true` なら続行、`false` で待機へ。
    fn prefetch_one(&mut self) -> bool {
        false
    }
}

/// engine + term + 提示状態を束ねた不純な実行器。[`RenderOp`] を実際に描画する。
struct Runner {
    /// reload（再コンパイル）のために所有する Typst World。
    world: PaladocsWorld,
    /// 現在のコンパイル結果（ラスタ/PDF の派生元）。
    compiled: CompiledDeck,
    deck: Deck,
    state: PresentState,
    presenter: Presenter<KittyBackend>,
    viewport: Viewport,
    /// 部分更新 diff 用に保持する「現在表示中フレーム」。
    cur_frame: Option<Frame>,
    /// ラスタ済みフレームの LRU キャッシュ（再ラスタ回避・アイドル先読み用）。
    cache: FrameCache,
}

impl Runner {
    fn new(world: PaladocsWorld, compiled: CompiledDeck, viewport: Viewport) -> Self {
        let deck = compiled.deck.clone();
        Self {
            world,
            compiled,
            deck,
            state: PresentState::start(),
            presenter: Presenter::new(KittyBackend, viewport),
            viewport,
            cur_frame: None,
            cache: FrameCache::new(cache_cap_bytes(viewport)),
        }
    }

    /// 起動時の初期提示（フレーム 0）。空デッキなら何もしない。
    fn present_initial(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        if self.deck.frame_count() == 0 {
            return Ok(());
        }
        if let Some(frame) = self.render(FrameId(0)) {
            self.presenter.present_slide(sink, &frame)?;
            self.cur_frame = Some(frame);
        }
        Ok(())
    }

    /// 1 つの [`Action`] を処理する。`Ok(false)` で終了要求。
    fn handle(&mut self, sink: &mut dyn Write, action: Action) -> io::Result<bool> {
        match action {
            Action::Quit => return Ok(false),
            Action::Resize(vp) => {
                self.viewport = vp;
                // 1 フレームのバイト数が変わるので容量を再計算（古い高解像度
                // エントリが残り続けないように即時淘汰）。
                self.cache.set_cap(cache_cap_bytes(vp));
            }
            Action::Reload => {
                if let Err(e) = self.reload() {
                    // 失敗時は直近の正常状態を保持し、診断のみ出す。
                    let _ = diag::report_engine_error(&mut io::stderr(), &e);
                    return Ok(true);
                }
            }
            _ => {}
        }
        let ops = step(&mut self.state, &self.deck, action);
        for op in ops {
            self.exec(sink, op)?;
        }
        Ok(true)
    }

    /// 変更ソースを再読込し、再コンパイルして Deck を作り直す。
    ///
    /// `World` のファイルキャッシュを stale 化してから `compile_deck` を呼び直し、
    /// `CompiledDeck` を差し替える（`comemo` の増分メモ化で未変更分は省かれる）。
    fn reload(&mut self) -> Result<(), paladocs_typst::EngineError> {
        self.world.reset_files();
        self.compiled = compile_deck(&self.world)?;
        self.deck = self.compiled.deck.clone();
        // 再コンパイルで同 FrameId が別内容になりうるため、キャッシュを全破棄。
        self.cache.clear();
        Ok(())
    }

    /// 1 つの [`RenderOp`] を engine + term で実行する。
    fn exec(&mut self, sink: &mut dyn Write, op: RenderOp) -> io::Result<()> {
        match op {
            RenderOp::PresentBase(f) => {
                if let Some(frame) = self.render(f) {
                    self.presenter.present_slide(sink, &frame)?;
                    self.cur_frame = Some(frame);
                }
            }
            RenderOp::ApplyOverlay { to, z, .. } => {
                if let Some(next) = self.render(to) {
                    match self.cur_frame.take() {
                        Some(prev) => {
                            self.presenter.apply_overlay(sink, &prev, &next, z)?;
                        }
                        // 現フレーム未保持なら安全側で全提示にフォールバック。
                        None => self.presenter.present_slide(sink, &next)?,
                    }
                    self.cur_frame = Some(next);
                }
            }
            RenderOp::RetreatOverlay => {
                self.presenter.retreat(sink)?;
                // 次の diff 基準を現フレームへ更新（再提示はしない）。
                if let Some(frame) = self.render(self.state.cur) {
                    self.cur_frame = Some(frame);
                }
            }
            RenderOp::Rerender => {
                if let Some(frame) = self.render(self.state.cur) {
                    self.presenter.resize(sink, self.viewport, &frame)?;
                    self.cur_frame = Some(frame);
                }
            }
            RenderOp::Noop => {}
        }
        Ok(())
    }

    /// 現 viewport で `frame` を取得する。キャッシュにあれば再利用し、無ければ
    /// ラスタしてキャッシュへ入れる。失敗は診断して `None`。
    fn render(&mut self, frame: FrameId) -> Option<Frame> {
        let size = self.viewport.pixel_size();
        let key = Key {
            id: frame,
            w: size.w,
            h: size.h,
        };
        if let Some(f) = self.cache.get(key) {
            return Some(f.clone());
        }
        match self.compiled.render_fit(frame, size) {
            Ok(f) => {
                self.cache.insert(key, f.clone());
                Some(f)
            }
            Err(e) => {
                let _ = diag::report_engine_error(&mut io::stderr(), &e);
                None
            }
        }
    }

    /// アイドル時に先読み候補を 1 枚だけラスタしてキャッシュを温める。
    ///
    /// まだキャッシュに無い最優先の 1 枚をラスタしたら `true`、候補が全て
    /// キャッシュ済み（または無し）なら `false` を返す。`false` を返すことで
    /// 呼び出し側はブロッキング待機へ移れる（CPU スピン回避）。端末へは書かない。
    fn prefetch_one(&mut self) -> bool {
        let size = self.viewport.pixel_size();
        let target = nav::prefetch_targets(&self.state, &self.deck)
            .into_iter()
            .find(|&f| {
                !self.cache.contains(Key {
                    id: f,
                    w: size.w,
                    h: size.h,
                })
            });
        match target {
            // ラスタ成功でキャッシュへ入れば true（次の候補へ）。失敗時は false を
            // 返して待機へ移す。失敗フレームはキャッシュに入らず再選択され続けるため、
            // true を返すと同じ失敗で CPU スピンしてしまう。
            Some(f) => self.render(f).is_some(),
            None => false,
        }
    }
}

// 既存 [`Runner`] のメソッドはこの trait とシグネチャ一致のため、薄い委譲で画像経路を
// 無改変のまま [`Stage`] へ載せる。
impl Stage for Runner {
    fn present_initial(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        Runner::present_initial(self, sink)
    }
    fn handle(&mut self, sink: &mut dyn Write, action: Action) -> io::Result<bool> {
        Runner::handle(self, sink, action)
    }
    fn prefetch_one(&mut self) -> bool {
        Runner::prefetch_one(self)
    }
}

/// cell-mode（ANSI セル＝MDPT 表示）の不純な実行器。
///
/// ナビ state machine（[`step`]）は画像経路と共有し、各 step 変化を「現 step を letterbox
/// 込み full グリッドへ投影 → 直前グリッドとの diff」で描画する（ブリーフ §ループ結線）。
/// overlay/retreat の z 管理は不要で、full グリッドの [`CellSink::draw_diff`] が部分更新を
/// 自然に賄う。アイドル先読みキャッシュは持たない（v1）。
struct CellRunner {
    /// reload（再コンパイル）のために所有する Typst World。
    world: PaladocsWorld,
    /// 現在のコンパイル結果。
    compiled: CompiledDeck,
    deck: Deck,
    state: PresentState,
    viewport: Viewport,
    /// diff 基準となる「現在表示中の full グリッド」。
    prev: Option<CellGrid>,
}

impl CellRunner {
    fn new(world: PaladocsWorld, compiled: CompiledDeck, viewport: Viewport) -> Self {
        let deck = compiled.deck.clone();
        Self {
            world,
            compiled,
            deck,
            state: PresentState::start(),
            viewport,
            prev: None,
        }
    }

    /// 現 viewport・現 step を letterbox 込み full グリッド（dims == 端末 `(cols,rows)`）へ
    /// 投影する。失敗は診断して `None`。
    fn render_full(&mut self) -> Option<CellGrid> {
        let cols = self.viewport.cols.min(u16::MAX as u32) as u16;
        let rows = self.viewport.rows.min(u16::MAX as u32) as u16;
        let w_pt = self.deck.meta.page_pt.w as f64;
        let h_pt = self.deck.meta.page_pt.h as f64;
        let lb = letterbox::text_grid(cols, rows, w_pt, h_pt, self.compiled.body_size_pt);
        let opts = RenderOpts {
            cols: lb.icols,
            rows: lb.irows,
            pixel_per_pt: letterbox::ppp_for(lb.irows, h_pt),
        };
        match render_step(&self.compiled, self.state.cur.0 as usize, &opts) {
            Ok(inner) => Some(letterbox::compose_full(
                &inner, cols, rows, lb.off_col, lb.off_row, DEFAULT_BG,
            )),
            Err(e) => {
                let _ = diag::report_engine_error(&mut io::stderr(), &e);
                None
            }
        }
    }

    /// 変更ソースを再読込して Deck を作り直す（画像 Runner と同経路）。
    fn reload(&mut self) -> Result<(), paladocs_typst::EngineError> {
        self.world.reset_files();
        self.compiled = compile_deck(&self.world)?;
        self.deck = self.compiled.deck.clone();
        Ok(())
    }

    /// 現 step の full グリッドを描画する。`full_redraw` 時は [`CellSink::draw_full`]、
    /// それ以外は直前グリッドとの [`CellSink::draw_diff`]（`prev` 無しは全描画）。
    /// いずれも diff 基準 `prev` を更新する。
    fn emit(&mut self, sink: &mut dyn Write, full_redraw: bool) -> io::Result<()> {
        let Some(grid) = self.render_full() else {
            return Ok(());
        };
        let mut cs = CellSink::new(&mut *sink);
        match (&self.prev, full_redraw) {
            (Some(prev), false) => cs.draw_diff(prev, &grid, (1, 1))?,
            _ => cs.draw_full(&grid, (1, 1))?,
        }
        self.prev = Some(grid);
        Ok(())
    }
}

impl Stage for CellRunner {
    fn present_initial(&mut self, sink: &mut dyn Write) -> io::Result<()> {
        if self.deck.frame_count() == 0 {
            return Ok(());
        }
        self.emit(sink, true)
    }

    fn handle(&mut self, sink: &mut dyn Write, action: Action) -> io::Result<bool> {
        match action {
            Action::Quit => return Ok(false),
            Action::Resize(vp) => {
                self.viewport = vp;
                // letterbox 再計算 → 再レンダ → 全描画 → diff 基準を新 full へ更新。
                step(&mut self.state, &self.deck, action);
                self.emit(sink, true)?;
                return Ok(true);
            }
            Action::Reload => {
                if let Err(e) = self.reload() {
                    // 失敗時は直近の正常状態を保持し、診断のみ出す。
                    let _ = diag::report_engine_error(&mut io::stderr(), &e);
                    return Ok(true);
                }
            }
            _ => {}
        }
        // step が cur を進め、その full グリッドを直前との diff で描画する。
        // 同一 step（Noop 等）は new==prev で diff 空＝出力空になる。
        step(&mut self.state, &self.deck, action);
        self.emit(sink, false)?;
        Ok(true)
    }
}

/// `present`: キーボード対話プレゼン。
pub fn run_present(root: &Path, mode: Mode) -> Result<(), CliError> {
    run(root, None, mode)
}

/// `preview`: キー入力。`control` が `Some` なら制御 socket（Neovim 契約）も併用。
pub fn run_preview(root: &Path, control: Option<&Path>, mode: Mode) -> Result<(), CliError> {
    run(root, control, mode)
}

/// プレゼンタ本体。`present` と `preview` は入力源が違うだけで共有する。
///
/// 出口モードは起動時 1 回 [`resolve_mode`] で決め、以降ナビでは不変（resize は
/// 再レンダのみ）。端末所有・panic hook・enter/leave 経路は image/cell 共通で、
/// 出口だけ [`Runner`] / [`CellRunner`] を差し替えて [`drive`] に渡す。
fn run(root: &Path, control: Option<&Path>, mode: Mode) -> Result<(), CliError> {
    let world = match PaladocsWorld::new(root) {
        Ok(w) => w,
        Err(e) => {
            diag::report_engine_error(&mut io::stderr(), &e).ok();
            return Err(CliError::Reported);
        }
    };
    let compiled = match compile_deck(&world) {
        Ok(c) => c,
        Err(e) => {
            diag::report_engine_error(&mut io::stderr(), &e).ok();
            return Err(CliError::Reported);
        }
    };

    // --- 出口モード決定（起動時 1 回）---
    let output = resolve_mode(mode);
    if should_warn_truecolor(output, detect_truecolor()) {
        // 警告のみ。出力は truecolor のまま（raw 化前なので素の stderr に出る）。
        eprintln!(
            "paladocs: COLORTERM does not advertise truecolor; cell-mode emits 24-bit color regardless (may look wrong on 256-color terminals)"
        );
    }

    // --- 端末所有を獲得 ---
    crossterm::terminal::enable_raw_mode().map_err(CliError::Io)?;
    install_panic_hook();
    let mut out = io::stdout();
    enter_sequence(&mut out).map_err(CliError::Io)?;
    // drop（panic unwind 含む）で「全画像削除→カーソル表示→alt-screen 退出→SGR
    // リセット→raw 解除」。cell-mode の leave も同梱される。
    let _guard = RawGuard::new(io::stdout(), true);

    let viewport = measure_viewport().map_err(CliError::Io)?;
    match output {
        OutputMode::Image => drive(Runner::new(world, compiled, viewport), out, control),
        OutputMode::Cell => drive(CellRunner::new(world, compiled, viewport), out, control),
    }
    // ここで `_guard` が drop → 端末復元。
}

/// 出口に依らないメインループ。入力多重化（キー + resize + socket）と
/// prefetch/blocking 待機を回し、各 [`Action`] を `stage` に処理させる。
fn drive(
    mut stage: impl Stage,
    mut out: io::Stdout,
    control: Option<&Path>,
) -> Result<(), CliError> {
    stage.present_initial(&mut out).map_err(CliError::Io)?;
    // 初期フレームを即時表示する。これが無いと最初の入力でメインループが
    // 1 周するまで出力がバッファに留まり、1 ページ目が出ない。
    out.flush().map_err(CliError::Io)?;

    // --- 入力多重化 ---
    let (tx, rx) = mpsc::channel::<Action>();
    spawn_input_thread(tx.clone());
    if let Some(sock) = control {
        spawn_control_thread(sock, tx)?;
    } else {
        drop(tx);
    }

    // --- メインループ ---
    // 入力が無い間は先読み候補を 1 枚ずつ温め、対象が尽きたらブロッキング待機する
    // （CPU スピンを避ける）。cell 出口は先読みを持たないため即待機へ移る。
    loop {
        let action = match rx.try_recv() {
            Ok(a) => a,
            Err(TryRecvError::Empty) => {
                if stage.prefetch_one() {
                    continue; // 1 枚温めたら入力を再チェック
                }
                match rx.recv() {
                    Ok(a) => a,
                    Err(_) => break, // 全 Sender drop = 終了
                }
            }
            Err(TryRecvError::Disconnected) => break,
        };
        match stage.handle(&mut out, action) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => return Err(CliError::Io(e)),
        }
        out.flush().map_err(CliError::Io)?;
    }

    Ok(())
}

/// 端末イベント（キー + resize）を [`Action`] チャネルへ流すスレッドを起こす。
fn spawn_input_thread(tx: Sender<Action>) {
    thread::spawn(move || {
        loop {
            match crossterm::event::read() {
                Ok(Event::Key(key)) => {
                    if key.kind == KeyEventKind::Release {
                        continue; // 押下/リピートのみ扱う（Windows の Release 重複回避）。
                    }
                    if let Some(action) = map_key(key.code, key.modifiers) {
                        let quit = action == Action::Quit;
                        if tx.send(action).is_err() || quit {
                            break;
                        }
                    }
                }
                Ok(Event::Resize(_, _)) => {
                    if let Ok(vp) = measure_viewport()
                        && tx.send(Action::Resize(vp)).is_err()
                    {
                        break;
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    });
}

/// 制御 socket（行区切り JSON）を [`Action`] チャネルへ流すスレッドを起こす。
///
/// Unix プラットフォームのみ。`UnixListener` で bind し、各接続の各行を
/// [`crate::protocol::parse_command`] で解析する。不正行は無視。
#[cfg(unix)]
fn spawn_control_thread(socket: &Path, tx: Sender<Action>) -> Result<(), CliError> {
    use std::io::BufRead;
    use std::os::unix::net::UnixListener;

    // 既存の socket ファイルを掃除してから bind。
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket).map_err(CliError::Io)?;
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let reader = io::BufReader::new(stream);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                match crate::protocol::parse_command(&line) {
                    Some(action) => {
                        let quit = action == Action::Quit;
                        if tx.send(action).is_err() || quit {
                            return;
                        }
                    }
                    None => {
                        // 不正行は無視 + ログ（stderr は端末を汚さない別ストリーム）。
                        eprintln!("paladocs: ignoring malformed control line: {line:?}");
                    }
                }
            }
        }
    });
    Ok(())
}

/// 非 Unix では制御 socket 非対応。
#[cfg(not(unix))]
fn spawn_control_thread(_socket: &Path, _tx: Sender<Action>) -> Result<(), CliError> {
    Err(CliError::Usage(
        "control socket (--control) is only supported on Unix platforms".to_string(),
    ))
}

/// 端末復元を panic unwind でも確実に走らせる panic hook を設置する。
///
/// [`RawGuard`] の Drop が主経路だが、保険として hook でも復元してから既定 hook へ。
fn install_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let mut out = io::stdout();
        let _ = crate::restore::restore_sequence(&mut out);
        let _ = crossterm::terminal::disable_raw_mode();
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_advance_variants() {
        for code in [
            KeyCode::Right,
            KeyCode::Char(' '),
            KeyCode::Enter,
            KeyCode::PageDown,
            KeyCode::Char('j'),
            KeyCode::Char('l'),
            KeyCode::Char('n'),
        ] {
            assert_eq!(map_key(code, KeyModifiers::NONE), Some(Action::Advance));
        }
    }

    #[test]
    fn key_retreat_variants() {
        for code in [
            KeyCode::Left,
            KeyCode::Backspace,
            KeyCode::PageUp,
            KeyCode::Char('k'),
            KeyCode::Char('h'),
            KeyCode::Char('p'),
        ] {
            assert_eq!(map_key(code, KeyModifiers::NONE), Some(Action::Retreat));
        }
    }

    #[test]
    fn key_slide_nav_and_home() {
        assert_eq!(
            map_key(KeyCode::Down, KeyModifiers::NONE),
            Some(Action::NextSlide)
        );
        assert_eq!(
            map_key(KeyCode::Up, KeyModifiers::NONE),
            Some(Action::PrevSlide)
        );
        assert_eq!(
            map_key(KeyCode::Home, KeyModifiers::NONE),
            Some(Action::Goto(FrameId(0)))
        );
    }

    #[test]
    fn key_reload_and_quit() {
        assert_eq!(
            map_key(KeyCode::Char('r'), KeyModifiers::NONE),
            Some(Action::Reload)
        );
        assert_eq!(
            map_key(KeyCode::Char('q'), KeyModifiers::NONE),
            Some(Action::Quit)
        );
        assert_eq!(
            map_key(KeyCode::Esc, KeyModifiers::NONE),
            Some(Action::Quit)
        );
        assert_eq!(
            map_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Action::Quit)
        );
    }

    #[test]
    fn unmapped_key_is_none() {
        assert_eq!(map_key(KeyCode::Char('z'), KeyModifiers::NONE), None);
        assert_eq!(map_key(KeyCode::Tab, KeyModifiers::NONE), None);
    }
}
