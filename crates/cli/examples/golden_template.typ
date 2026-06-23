// golden_template.typ — Paladocs の受け入れデッキ（Touying 非依存・素の #page ベース）。
//
// 画像経路（Typst → 正準 RGBA8 → ターミナル blit）のフィデリティを確認するための
// 高精細スライド。pt 基準の φ タイポグラフィ、tiling() 地、place() のサブセル配置、
// α 合成、数式を使う。pdfpc メタを持たないため Paladocs はフォールバック構築で
// 9 スライド × 1 step のデッキにする。
//
// フォント: Archivo / Noto Sans JP / DejaVu Sans Mono を指定するが、未インストール時は
// Typst のフォント探索が代替へフォールバックする（警告はエンジンが破棄）。画像経路
// なので端末表示フォントには依存しない。

// --- 黄金比キャンバス（φ ≈ 1.618） ---
#let phi = 1.618
#let unit = 9cm
#set page(width: unit * phi, height: unit, margin: 0pt, fill: rgb("#0e1116"))
#set text(font: ("Archivo", "Noto Sans JP"), fill: rgb("#e8eaed"), size: 24pt)
#set par(leading: 0.8em)

#let accent = rgb("#f5c044")
#let muted = rgb("#9aa0a6")

// φ で分割したマージン枠の中に本文を置くヘルパ。
#let frame(body) = block(
  width: 100%,
  height: 100%,
  inset: (x: unit / phi / 2, y: unit / phi / 3),
)[#body]

// 各スライド共通のフッタ（place でサブセル位置に固定）。
#let footer(n) = place(
  bottom + right,
  dx: -1.2cm,
  dy: -0.8cm,
  text(size: 14pt, fill: muted)[Paladocs · #n / 9],
)

// === 1. タイトル ===
#frame[
  #place(top + left, dx: 0pt, dy: 0pt, rect(width: 6pt, height: 100%, fill: accent))
  #v(1fr)
  #text(size: 64pt, weight: "bold")[Paladocs]
  #v(0.2em)
  #text(size: 28pt, fill: muted)[Typst 専用スライドプレゼンタ — 画像経路レンダリング]
  #v(1fr)
]
#footer(1)
#pagebreak()

// === 2. 黄金比の作図（place + 矩形） ===
#frame[
  #text(size: 36pt, weight: "bold")[黄金比 φ = #calc.round(phi, digits: 3)]
  #v(0.5em)
  #box(width: 100%, height: 60%)[
    #place(left + horizon, rect(width: 38%, height: 100%, fill: accent.transparentize(60%), stroke: accent))
    #place(left + horizon, dx: 38%, rect(width: 23.5%, height: 100%, fill: accent.transparentize(75%), stroke: accent))
    #place(left + horizon, dx: 61.5%, rect(width: 14.5%, height: 100%, stroke: muted))
  ]
]
#footer(2)
#pagebreak()

// === 3. CJK タイポグラフィ（Noto Sans JP フォールバック） ===
#frame[
  #text(size: 36pt, weight: "bold")[多言語タイポグラフィ]
  #v(0.4em)
  日本語のグリフも Typst コンパイル時にフォント解決される。端末側の
  2 フォントスタック問題は画像経路では消える。
  #v(0.3em)
  #text(fill: accent)[漢字・ひらがな・カタカナ — ABCdef 0123]
]
#footer(3)
#pagebreak()

// === 4. 数式 ===
#frame[
  #text(size: 36pt, weight: "bold")[数式]
  #v(0.6em)
  #set text(size: 28pt)
  $ phi = (1 + sqrt(5)) / 2 approx 1.618 $
  #v(0.4em)
  $ F_n = (phi^n - (-phi)^(-n)) / sqrt(5), quad lim_(n -> oo) F_(n+1) / F_n = phi $
]
#footer(4)
#pagebreak()

// === 5. α 合成（半透明の重なり） ===
#frame[
  #text(size: 36pt, weight: "bold")[アルファ合成]
  #v(0.4em)
  #box(width: 100%, height: 55%)[
    #place(center + horizon, dx: -3cm, circle(radius: 3cm, fill: rgb(245, 80, 80, 150)))
    #place(center + horizon, circle(radius: 3cm, fill: rgb(80, 200, 120, 150)))
    #place(center + horizon, dx: 3cm, circle(radius: 3cm, fill: rgb(90, 140, 245, 150)))
  ]
]
#footer(5)
#pagebreak()

// === 6. tiling() 地紋 ===
#let dots = tiling(size: (26pt, 26pt))[
  #place(center + horizon, circle(radius: 2.2pt, fill: accent.transparentize(40%)))
]
#frame[
  #text(size: 36pt, weight: "bold")[タイル地紋]
  #v(0.4em)
  #rect(width: 100%, height: 55%, fill: dots, stroke: muted, radius: 6pt)
]
#footer(6)
#pagebreak()

// === 7. 等幅コード（DejaVu Sans Mono フォールバック） ===
#frame[
  #text(size: 36pt, weight: "bold")[等幅レンダリング]
  #v(0.4em)
  #block(
    width: 100%,
    inset: 16pt,
    radius: 6pt,
    fill: rgb("#161a21"),
    text(font: "DejaVu Sans Mono", size: 20pt, fill: rgb("#a6e3a1"))[
      `fn scale_for(page_pt: SizePt, vp: PixelSize) -> f32`
    ],
  )
]
#footer(7)
#pagebreak()

// === 8. サブセル配置（place のオフセット） ===
#frame[
  #text(size: 36pt, weight: "bold")[サブセル配置]
  #v(0.4em)
  #box(width: 100%, height: 55%)[
    #for i in range(6) {
      place(top + left, dx: i * 3.2cm + 0.5cm, dy: i * 1.4cm + 0.5cm,
        rect(width: 2.6cm, height: 1.1cm, radius: 4pt,
          fill: accent.transparentize(i * 12%), stroke: accent))
    }
  ]
]
#footer(8)
#pagebreak()

// === 9. クロージング ===
#frame[
  #v(1fr)
  #align(center)[
    #text(size: 48pt, weight: "bold")[ありがとうございました]
    #v(0.3em)
    #text(size: 24pt, fill: muted)[Paladocs — fidelity by the image path]
  ]
  #v(1fr)
]
#footer(9)
