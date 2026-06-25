# CLAUDE.md — Paladocs

このファイルはセッション開始時に自動で読まれる**恒久的な開発規律**。個々のタスクの詳細仕様は別途のブリーフ（`*-brief.md`）にあり、本書はそれらに**横断して常に適用**される。

---

## プロジェクト概要

Paladocs は **Typst 専用**のスライドプレゼンタ／ライブラリ。Typst ソースをスライドとして GPU ターミナル（Knightty）に描画し、PDF も出力する。エンジンは Typst 固定（**LaTeX 非対応**）。

---

## ワークスペース構成と依存方向

```
core   ← 依存なし（std のみ）。論理 IR。解像度非依存・pixels を持たない。
render ← core。pixels/幾何の型・raster-diff・合成・letterbox。I/O なし。
typst  ← render, core, + Typst コンパイラ群。重い外部依存はここだけ。
term   ← render, core。Knightty へ placement/z/delete で送る。
cli    ← 上記すべて。プレゼンタループ + 制御 socket。
```

**鉄則**
- 依存はこの順で「下」へのみ。`core`/`render` は上位（typst/term/cli）に依存しない。
- **pixels を `core` に入れない。** 論理（core）と画素（render）は別層。
- Typst 型・端末型・GPU を上位へ漏らさない。重い外部依存は `typst` クレートに隔離し、`core`/`render` の依存を増やさない。

---

## 開発フロー：テストファースト（TDD）— 必須

**実装の前にテストを書く。** 例外を作らない。

1. ブリーフの型・不変条件・テスト表を、まず**失敗するテスト**として書く。
2. `cargo test` で**赤**を確認する（テストが本当に効いていることの確認）。
3. テストを**緑**にする最小実装を書く。
4. 緑を保ったままリファクタする。
5. **対応する失敗テストが無い実装変更を入れない。**

- 公開関数・各不変条件・各エラーケースに必ずテストを付ける。
- 境界条件（範囲外・不正入力・空入力）を明示的にテストする。
- 数値を含む結果（合成の混色など）は**具体値で固定**する。
- コミット/差分は、実装と同時かそれ以前にテストが入っていること。

---

## 横断契約（全クレートで不変）

- **正準ピクセル形式**: RGBA8 / **ストレート（非プリマルチプライド）アルファ** / row-major・左上原点・隙間なし / 合成は sRGB バイト空間。`render` で 1 箇所定義し doc 化。全クレートがこれに従う。
  - Typst（tiny-skia Pixmap）は**プリマルチプライド**。`typst` クレートで**必ずアンプリマルチプライ**して正準形式へ変換する。
- **IR 不変条件**: `Deck` は I1（slide.index == 位置）/ I2（steps 非空）/ I3（frame == 発表順通し番号）を満たす。**`Deck` を返す前に必ず `Deck::validate()` を通す。**
- **FrameId** はデッキ全体の発表順ページ番号。

---

## コーディング規約

- Rust `edition = "2024"`。
- **依存規律**: `core` は std のみ。`render` は `core` + std のみ。便利クレート（`image`/`thiserror` 等）を `core`/`render` に入れない。重い依存は `typst` に隔離。
- **パニックしない**（ライブラリコード）。範囲外・失敗は `Option`/`Result` で返す。
- 公開 API には doc コメント。**不変条件は doc に明記**し、暗黙にしない。
- エラー型は純粋クレートでは手書き（`Display` + `std::error::Error`）。
- **勝手に型・フィールド・機能を増やさない。** ブリーフの形を保ち、成長点だけ残す。
- スコープを越えない。担当クレート外（下流の関心事）を「気を利かせて」実装しない。

---

## 外部 API・スキーマの扱い（うろ覚え禁止）

- Typst コンパイラ等の**シグネチャ・型・スキーマを記憶で決め打ちしない**。実装時に **docs.rs で確認**する。
- 構造が不明なデータ（例: Touying の pdfpc メタ）は、**実物を 1 回コンパイルしてダンプし、実測してから**パーサを書く。スナップショットを残す。
- `typst-*` 群は**バージョン・ロックステップ**で固定し、Touying のバージョンも Typst 版に合わせる。

---

## 完了の定義（検証コマンド）

以下が**全て**通って初めて完了。clippy 警告ゼロは**ハードゲート**。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
cargo doc --workspace --no-deps
```

- ネットワーク依存テスト（Typst パッケージ取得など）は feature/`#[ignore]` で分離し、CI ではキャッシュ済みのみ緑にする。

---

## タスク仕様書

詳細仕様はタスクごとの別ブリーフにある（IR / render / typst / term / cli …）。本書と矛盾する場合は**本書の横断規律を優先**し、齟齬は報告する。

---

## 現状（クレートが進むたびに更新する）

- `core` — 実装済み（IR・validate・ナビ・テスト緑）
- `render` — 実装済み（型・diff・合成・letterbox・ビューポート連動スケール `scale_for`(pixels-per-point)・セル層 `CellGrid`/半ブロック量子化/罫線描画 `draw_box`・`draw_hline`・`draw_vline`/端末既定色番兵 `DEFAULT`(a=0→SGR `39`/`49`)・複数セル拡大テキスト型 `CellSpan`・テスト緑）
- `typst` — 実装済み（M1: World/compile/render(ストレートアルファ)/render_fit/PDF/reload/診断・テスト緑。M2: pdfpc メタ実測→overlay/ノートのグルーピング。Touying テストは `#[ignore]`／ネットワーク分離）
- `term` — 実装済み（Kitty graphics protocol: 送信(`a=t,f=32`)/placement(`a=p`)/削除を分離・base64 手書き・チャンク ≤4096・配置写像 `place_geometry`・Presenter ライフサイクル(present/overlay 部分更新/retreat ソフト `d=i`/clear ハード `d=I`/resize)・リーク無し・テスト緑。実機 round-trip は `#[ignore]`。Knightty の `d=p` はセル削除のため単一 placement 削除に `d=i`+`p=` を採用。送出 medium は純関数 `select_medium`(capability×サイズ→`Medium`)で選択、参照 wire は `transmit_reference`(`t=s`/`t=f`)。Knightty 実測(`crates/proto`: 非 `t=d` は `UnsupportedFeature`)に基づき `KittyBackend` は常に直送(`t=d`)、参照 medium は確保機構を持つ将来 backend 向けの建材。cell 出口は `CellSink`(CellGrid→ANSI)＋見出し拡大の `draw_spans`(Knightty OSC 7777 `ESC]7777;knightty;span=CxR:TEXT ST` を grid 描画後に重ねる)）
- `cli` — 実装済み（バイナリ `paladocs`。サブコマンド present/preview/build。純粋: ナビ状態機械 `nav::step`・socket JSON/`CSI 16 t` パーサ `protocol`・引数解析 `cli::parse_args`・診断 JSON `diag`・端末復元シーケンス `restore`・キー写像 `app::map_key`。不純: 端末所有(raw/alt-screen/カーソル・panic 安全な復元 `RawGuard`+panic hook)・viewport 計測(`crossterm::window_size`→セル寸法。`CSI 16 t` はフォールバック parser のみ)・入力多重化(キー+resize スレッド / `--control` の `UnixListener` は Unix 限定)・engine 再ラスタ→term 送出。スライド境界は clear+全提示、部分更新はスライド内 forward のみ。`preview` の `--control` socket は任意化(省略時はキー入力のみの表示)。`--mode auto|image|cell` で出口レンダラを選択(既定 auto=画像経路、cell で ANSI セル経路を強制)。cell 経路は**意味的 TUI 投影**(`typst` の `render_step`): 地色は端末既定で透過、`stroke` 付き `Shape` のみ描画(`Rect`→アウトライン罫線・`Line`→横/縦罫線、塗りのみ装飾矩形は非描画)、テキストは**各グリフの絶対比例位置**(`x_pt/pt_per_col`)で端末既定色の鮮明セルとして配置(`place_runs`、run 内/run 間とも同一基準で Typst レイアウトを忠実投影・BOLD ティア維持・余白は埋めず透過)。半ブロックモザイクは使わず二重描画は原理的に無し(`Curve`/`Image` は v1 非描画)。cell グリッド寸法は本文サイズ基準(`letterbox::text_grid`: `pt_per_col≈body/2`・`pt_per_row≈body` で欧文≒1セル/CJK≒2セルに密詰め、テキスト無しはアスペクト `letterbox` へフォールバック)。見出しの実寸拡大は Knightty の cell span(OSC 7777)で実現: `render_step` が見出し run(`round(size/body)>=2`)を `CellSpan` として返し(`StepRender{grid,spans}`)、cli が grid 描画後に `draw_spans` で重ねる。見出しは grid にも通常セルで残すため非対応端末では通常サイズで出る(graceful fallback)。既定 ON、`--no-cell-spans` で無効。pdfpc 無しのプレーンデッキは無音フォールバックせず `PdfpcMissing`(typst 設計と整合)。テスト緑。実機 PTY+Knightty round-trip と socket 実 I/O は `#[ignore]`。追加依存 `crossterm`/`serde_json` は cli 限定）
