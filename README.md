# Paladocs

Typst ソースをスライドとして端末（Knightty）に描画し、PDF も出力する Rust 製ライブラリ。エンジンは **Typst 固定**（LaTeX 非対応）。

## クレート構成

以下のクレート群へ分割される。`core` / `render` / `typst` は実装済み。

| クレート | 役割 | 状態 |
|---|---|---|
| `paladocs-core` | 論理 IR（解像度非依存・I/O フリー）。 | 実装済み |
| `paladocs-render` | 正準 RGBA・raster-diff・合成・letterbox の pixel プリミティブ（I/O フリー）。 | 実装済み |
| `paladocs-typst` | Typst ソースをコンパイルして `Deck` を組み立て、フレームを描画し PDF を出力する。重い外部依存を隔離。 | 実装済み |
| `paladocs-term` | 端末（Knightty 等）へ描画する。 | 未着手 |
| `paladocs-cli` | コマンドラインエントリポイント。 | 未着手 |

## `paladocs-core`

`crates/core` に実装。Typst 出力に対応する論理スライドデッキの **IR とナビゲーション API** だけを提供する。

- pixels・GPU・Typst コンパイラ・端末プロトコルを持たない
- 外部 crate に依存しない（`std` のみ）
- 公開 API は範囲外でパニックせず `Option` / `Result` を返す
- `Deck::validate` が不変条件（I1〜I3）を検査する

詳細は `docs/paladocs-core-ir-brief.md` を参照。

## `paladocs-typst`

`crates/typst` に実装。Typst コンパイラ群（`typst` / `typst-render` / `typst-pdf` /
`typst-kit`、0.15 系ロックステップ）を**このクレートだけ**に隔離する。

- `Engine::compile(root)` で `root.typ` を 1 度コンパイルし、同じ `PagedDocument` から
  `Deck`・`Frame`・PDF をすべて派生させる。
- フレームは tiny-skia の **プリマルチプライド** Pixmap を**アンプリマルチプライ**し、
  `paladocs-render` の正準形式（RGBA8・ストレートアルファ）へ変換して返す。
- Deck は Touying の pdfpc メタデータから overlay/ノートを復元（M2）し、取れなければ
  「1 ページ = 1 スライド」へフォールバック（M1）。いずれも `Deck::validate` を通す。
- `reload()` で変更ソースを再コンパイル（`comemo` 増分メモ化）。
- コンパイルエラーは行/列つきの診断（`EngineError::Compile`）に変換する。

詳細は `docs/paladocs-typst-brief.md` を参照。

## 開発

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # Touying（ネットワーク）依存テストは #[ignore] で分離
cargo build --workspace
cargo doc --workspace --no-deps
```

Touying のグルーピング等、ネットワーク（`@preview/touying` 取得）依存のテストは
`#[ignore]` 付き。キャッシュ済み環境で次のように実行する:

```bash
cargo test -p paladocs-typst -- --ignored
```

## License

MIT
