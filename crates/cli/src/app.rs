//! 不純シェル: engine・term・端末・スレッド・socket を結線し、[`Action`] ストリームを
//! [`crate::nav::step`] で回して [`RenderOp`] を実行する。
//!
//! ここで初めて実際の端末・スレッド・socket・プロセス境界に触れる。決定ロジックは
//! [`crate::nav`] に純粋に隔離してあるので、本モジュールは orchestration に徹する。

use std::io::{self, Write};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::thread;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use paladocs_core::{Deck, FrameId};
use paladocs_render::Frame;
use paladocs_term::{KittyBackend, Presenter, Viewport};
use paladocs_typst::Engine;

use crate::diag;
use crate::error::CliError;
use crate::nav::{Action, PresentState, RenderOp, step};
use crate::restore::{RawGuard, enter_sequence};
use crate::terminal::measure_viewport;

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

/// engine + term + 提示状態を束ねた不純な実行器。[`RenderOp`] を実際に描画する。
struct Runner {
    engine: Engine,
    deck: Deck,
    state: PresentState,
    presenter: Presenter<KittyBackend>,
    viewport: Viewport,
    /// 部分更新 diff 用に保持する「現在表示中フレーム」。
    cur_frame: Option<Frame>,
}

impl Runner {
    fn new(engine: Engine, viewport: Viewport) -> Self {
        let deck = engine.deck().clone();
        Self {
            engine,
            deck,
            state: PresentState::start(),
            presenter: Presenter::new(KittyBackend, viewport),
            viewport,
            cur_frame: None,
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
            Action::Resize(vp) => self.viewport = vp,
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

    /// 再コンパイルして Deck を作り直す。
    fn reload(&mut self) -> Result<(), paladocs_typst::EngineError> {
        self.engine.reload()?;
        self.deck = self.engine.deck().clone();
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

    /// 現 viewport で `frame` を再ラスタする。失敗は診断して `None`。
    fn render(&self, frame: FrameId) -> Option<Frame> {
        match self.engine.render_fit(frame, self.viewport.pixel_size()) {
            Ok(f) => Some(f),
            Err(e) => {
                let _ = diag::report_engine_error(&mut io::stderr(), &e);
                None
            }
        }
    }
}

/// `present`: キーボード対話プレゼン。
pub fn run_present(root: &Path) -> Result<(), CliError> {
    run(root, None)
}

/// `preview`: 制御 socket（Neovim 契約）+ キー入力。
pub fn run_preview(root: &Path, control: &Path) -> Result<(), CliError> {
    run(root, Some(control))
}

/// プレゼンタ本体。`present` と `preview` は入力源が違うだけで共有する。
fn run(root: &Path, control: Option<&Path>) -> Result<(), CliError> {
    let engine = match Engine::compile(root) {
        Ok(e) => e,
        Err(e) => {
            diag::report_engine_error(&mut io::stderr(), &e).ok();
            return Err(CliError::Reported);
        }
    };

    // --- 端末所有を獲得 ---
    crossterm::terminal::enable_raw_mode().map_err(CliError::Io)?;
    install_panic_hook();
    let mut out = io::stdout();
    enter_sequence(&mut out).map_err(CliError::Io)?;
    // drop（panic unwind 含む）で「全画像削除→カーソル表示→alt-screen 退出→raw 解除」。
    let _guard = RawGuard::new(io::stdout(), true);

    let viewport = measure_viewport().map_err(CliError::Io)?;
    let mut runner = Runner::new(engine, viewport);
    runner.present_initial(&mut out).map_err(CliError::Io)?;

    // --- 入力多重化 ---
    let (tx, rx) = mpsc::channel::<Action>();
    spawn_input_thread(tx.clone());
    if let Some(sock) = control {
        spawn_control_thread(sock, tx)?;
    } else {
        drop(tx);
    }

    // --- メインループ ---
    for action in rx {
        match runner.handle(&mut out, action) {
            Ok(true) => {}
            Ok(false) => break,
            Err(e) => return Err(CliError::Io(e)),
        }
        out.flush().map_err(CliError::Io)?;
    }

    Ok(())
    // ここで `_guard` が drop → 端末復元。
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
