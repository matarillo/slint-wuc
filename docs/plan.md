# Slint ランタイムフォーク計画 — Windows.UI.Composition ビジュアルツリー専用ランタイム基盤

## Context

Slint 1.19.0 のランタイム (`reference/slint-1.19.0/internal/core`) は、`Platform` / `WindowAdapter` / `Renderer` / `ItemRenderer` の抽象化の上で、毎フレーム item tree を走査して描画する即時モード設計になっている。
これをフォークし、Windows 専用・Windows.UI.Composition のビジュアルツリー (保持モード) 専用のランタイム基盤に作り替える。
item tree のノードを Composition の `Visual` に対応させ、プロパティの変更を差分で Visual に反映する。

確定した前提:

- 対象 OS は **Windows 11 のみ** (ビルド 22621 (22H2) 以降。2026-11-19 に 22000 から上げた。Mica の公開 API が 22621 以降のため)。
- 互換目標は **生成コード互換**。上流コンパイラーが生成する Rust/C++ コードとインタープリターが参照する内部インターフェースを、ソースレベルで維持する。
- `.slint` コンパイラーは開発対象外。ただし無改造の上流コンパイラー (crates.io の `slint-build` 1.19.0) を**検証用ツール**として使う。
- バックエンド抽象・レンダラー抽象は全廃する。`reference/wasamo-*` の設計は参照しない。
- リポジトリは現状 `.gitignore` のみ (`reference/` は追跡対象外)。実質グリーンフィールド。

## 設計方針

### 残すもの / 捨てるもの / 新設するもの

| 区分 | 対象 (上流 `internal/core` 基準) |
|---|---|
| 残す (ほぼ無改造) | `properties*` `callbacks` `model*` `layout` `item_tree` `item_focus` `input` `animations*` `timers` `string` `sharedvector` `slice` `lengths` `graphics/{color,brush,border_radius,path}` `menus` `accessibility` `translations` `styled_text` `data_transfer` `component_factory` `rtti` `api` (公開型) |
| 作り替える | `items*` (`render` を Visual 更新に置換)、`window*` (`WindowInner` は維持し、ホストを直結)、`context` (イベントループ直結)、`graphics/image` (デコードは維持、描画先をサーフェスに) |
| 捨てる | `renderer` の描画系メソッド (`Renderer` トレイト自体はフォント登録とテキスト計測の継ぎ目として残す。Phase 0 で変更、`docs/design/compat-contract.md`) `item_rendering` (`ItemRenderer`) `partial_renderer` `platform` (`Platform` トレイトは公開 API から外し、コア内部の継ぎ目として残す。Phase 1 で変更。`WindowEvent` 等の型は残す) `textlayout*` `font_collection` (parley/fontique) `graphics/{wgpu_*,bitmapfont,boxshadowcache,rendering_metrics_collector}` `unsafe_single_threaded`、no_std / wasm / 他 OS の `cfg`、`slint_int_coord`、`internal/backends/*`、`internal/renderers/*` |
| 新設 | `host/` (Win32 メッセージループ、HWND、DPI)、`composition/` (Compositor、グラフィックスデバイス、サーフェス)、`visual_tree/` (同期エンジン)、`text/` (DirectWrite)、`testing` (ヘッドレス検証) |

### ビジュアルツリー同期エンジン (中核設計)

- **対応関係**: 1 item = 1 Visual を基本とする。`SpriteVisual` / `ShapeVisual` は `ContainerVisual` の派生なので、子 item の Visual を直接ぶら下げる。
- **状態の置き場**: items 構造体の `cached_rendering_data` フィールドは残し、ウィンドウごとの Visual ノード表へのハンドル (index + generation) に転用する。構造体に COM オブジェクトを持たせない。
- **差分更新**: 上流の `PartialRenderer::compute_dirty_regions` と同じく、ウィンドウのトラッカーが dirty のときだけ木を走査し、ノードごとの内容トラッカーとジオメトリ比較で変化した Visual だけを更新する。構造変化 (リピーター等) も走査で拾える。走査コストが Phase 2 の計測で約 1 ms を超えたら、dirty キュー方式に切り替える。(Phase 0 で変更。詳細は `docs/design/visual-tree.md`)
- **`ItemVTable`**: `render` スロットを `update_visual` に置換する。items が Composition API を直接呼ぶ (抽象層なし)。
- **構造変化**: `register_item_tree` / `unregister_item_tree` (`item_tree.rs:215,235`) と `ensure_instantiated` をフックし、親ノードを「子リスト dirty」にして Visual の子コレクションを突き合わせる。z 順は `visit_item_tree_z_sorted` の順序に従う。
- **フレーム駆動**: アニメーション中のみティックし、アイドル時は CPU ゼロ。Slint のプロパティアニメーションは UI スレッドのティックで駆動する (Composition アニメーションへの委譲は将来の最適化)。
- **ヒットテストと入力**: Visual ではなく、上流と同じく item tree のジオメトリで行う (`input.rs` を維持)。
- **未確定でスパイクが決めるもの**: 座標系 (ルートスケール一括か、Visual ごとの物理ピクセル丸めか)、フレーム信号源、角丸/枠線の Visual 種別、グループ不透明度の実現方法。

### 互換のためにコアで守る設計 (互換コントラクト)

1. クレート名 `i-slint-core` (`i_slint_core`) とモジュールパス・型名を維持する。インタープリターと `api/*` の将来移植を容易にする。
2. items 構造体は `#[repr(C)]` + `FieldOffsets` + `SlintElement` (rtti) を維持し、構造体名・フィールド名・型を `builtin_elements.rs` の `item!` 宣言と一致させる。
3. `ItemTreeVTable` (`item_tree.rs:43`) は無変更。`vtable` / `const-field-offset` は crates.io 版 (0.5.1 / 0.2.1) を使う。
4. 生成コードは `sp::Rc<dyn sp::WindowAdapter>` と `sp::WindowInner::from_pub(..)` を直接書く (`generator/rust.rs:740,790,802`)。`WindowAdapter` はトレイト名のまま**互換の継ぎ目**として残し、実装は 1 つだけにする。コア内部は具体型を直接使う。
5. `ffi` フィーチャー (`extern "C"` と cbindgen) と `rtti` フィーチャーを維持する。
6. テスト用 API (`i-slint-backend-testing` 相当: イベント注入、モック時刻) を同じ形で用意する。

明示的に非互換とするもの: `slint::platform` (独自バックエンド API)、ソフトウェア/FemtoVG/Skia レンダラー、`set_rendering_notifier` / `GraphicsAPI` / wgpu 連携、`BackendSelector`、no_std/MCU、`Native*` 要素 (Qt の native スタイル専用)。

### `element!` のカバー範囲

| 分類 | 要素 | 実現方法 | フェーズ |
|---|---|---|---|
| 基本 Visual | Empty, Rectangle, Window | `ContainerVisual` / `SpriteVisual` + HWND | 1 |
| 視覚属性 | Rectangle (枠線・角丸・グラデーション), Clip, Opacity, Layer, Transform | `ShapeVisual`, `Visual.Clip`, `LayerVisual`, 変換行列 | 2 |
| 動的構造 | ComponentContainer、リピーター/条件 (要素ではないが必須) | 子リスト突き合わせ | 2 |
| コンパイラーが下げる要素 | Row, GridLayout, VerticalLayout, HorizontalLayout, FlexboxLayout, PropertyAnimation, Tab, TabWidget, RadioButton, RadioGroup | ランタイム側は `layout.rs` / `properties` の関数のみ | 2 |
| 入力 | TouchArea, FocusScope, KeyBinding, Flickable, SwipeGestureHandler, ScaleRotateGestureHandler, WindowMoveArea, Timer | Win32 入力 → `WindowInner::process_*` | 3 |
| テキスト | Text, StyledText, TextInput, TextInputInterface | DirectWrite + D2D サーフェス、IME | 4 |
| 画像・図形・効果 | Image, Path (+MoveTo/LineTo/ArcTo/CubicTo/QuadraticTo/Close), BoxShadow, BackdropBlur | サーフェスブラシ、`CompositionPathGeometry`、エフェクトブラシ | 5 |
| ウィンドウ機能 | PopupWindow, Tooltip, TooltipArea, ContextMenuArea, ContextMenuInternal, MenuBar, Menu, MenuItem, MenuSeparator, Dialog, SystemTrayIcon, DragArea, DropArea, Platform | 子 HWND、Win32 メニュー、`Shell_NotifyIcon`、OLE D&D | 6 |
| 捨てる | NativeButton ほか `Native*` 全 16 要素 | — | — |

## ワークスペース構成

```
Cargo.toml                 ワークスペース (edition 2024、windows クレートは上流と同じ 0.62 系で固定)
crates/core/               i-slint-core のフォーク (host/ composition/ visual_tree/ text/ を内包)
crates/core-macros/        internal/core-macros のコピー
crates/common/             internal/common のコピー (enums, builtin_structs, key_codes)
crates/slint/              互換プローブ用ファサード (api/rs/slint の lib.rs + private_unstable_api.rs 由来)
spikes/s1-hosting/ …       スパイク実装 (本体に依存しない独立バイナリ)
tests/fixtures/            手書き ItemTree (生成コード相当)
tests/probe/               .slint + 上流 slint-build の生成コードを本ランタイムでビルドする検証
docs/design/               互換コントラクト、スパイク結果、設計判断
```

取り込みは「上流 1.19.0 の無改造コピー」を最初のコミットにし、以後の削除・改造を上流との差分として追えるようにする。SPDX ヘッダーは保持する。

## スパイク (windows クレート使用、各結果を `docs/design/spike-*.md` に記録)

| ID | 検証内容 | 決める設計 | 時期 |
|---|---|---|---|
| S1 ホスティング | HWND + `DispatcherQueueController` + `Compositor` + `DesktopWindowTarget`、リサイズ追従、DPI 変更、コミットのタイミング、フレーム信号源 (`DCompositionWaitForCompositorClock` / タイマー) | イベントループとフレーム駆動 | P0 |
| S2 プリミティブ | 矩形・角丸・枠線・グラデーションの `SpriteVisual` と `ShapeVisual` の比較、`ID2D1Geometry` を `IGeometrySource2DInterop` で包んだ `CompositionPath` (個別角丸・Path 用)、`Visual.Opacity` がグループ不透明度か、分数 DPI での座標系と文字の鮮明さ、Visual 1 万〜5 万個の生成時間とメモリ | Visual 種別、座標系、1:1 対応の妥当性 | P0 |
| S3 検証基盤 | `GraphicsCaptureItem::CreateFromVisual` によるピクセル取得、HWND なしでの Visual ツリー構築、ツリーのテキストダンプ | 自動テストの方式 | P0 |
| S4 テキスト | DirectWrite レイアウトの D2D 描画 → `CompositionDrawingSurface`、透過面でのアンチエイリアス、サーフェス再生成コスト、デバイスロスト、計測 API の対応 (`text_size` / カーソル矩形 / 位置→バイトオフセット)、IME (TSF か IMM32) | テキスト基盤 | P4 冒頭 |
| S5 エフェクト | Win2D なしで `IGraphicsEffect` + `IGraphicsEffectD2D1Interop` を Rust で実装 (ぼかし・タイル・着色)、`DropShadow` と事前ぼかし 9 分割サーフェスの比較、`BackdropBrush`、Mica | 画像の tiling/colorize、BoxShadow、BackdropBlur | P5 冒頭 |

## モデル分担 (Opus / Sonnet)

Fable は使わない。
メインセッションは **Opus** で動かし、設計判断・スパイク・中核実装・レビューを担う。
**Sonnet** の作業は、Opus のメインセッションから `Agent` (`model: "sonnet"`) に委譲する。
委譲時は、対象ファイル・従うべき雛形 (Opus が先に実装した 1 例)・完了条件 (ビルドとテスト) を指示に含める。
Sonnet の成果は Opus がレビューしてからコミットする。

| 担当 | 作業の性質 | 主な作業 |
|---|---|---|
| Opus | 設計を決める・前例のない実装 | 互換コントラクトの分類判断、全スパイク (S1〜S5)、同期エンジン (`visual_tree/`)、`host/` のイベントループとフレーム駆動、`ItemVTable` 改造、構造同期、各カテゴリの**最初の 1 item** (雛形)、テキスト・エフェクト基盤の中核、各フェーズのゲートレビュー |
| Sonnet | 決まった方針の機械的な適用・横展開 | 上流コードの取り込みと刈り込み、`cfg` 除去、ビルドエラー修正、シンボル棚卸しの下調べ、雛形に倣った item の横展開、Win32 キーコード等の対応表、上流テストの移植と除外リスト作成、rtti/ffi の網羅、ドキュメント整形 |

以下のフェーズ内の各項目に担当を `[Opus]` / `[Sonnet]` で示す。
Phase 0〜2 は Opus 中心、Phase 3 以降は Sonnet 中心 (各フェーズ冒頭の基盤とスパイクのみ Opus) になる。

## フェーズ

### Phase 0 — 土台・互換コントラクト・スパイク (Opus 中心)
- [Sonnet] ワークスペース作成、上流コードの無改造コピーによる取り込み (上記構成)、SPDX ヘッダー保持の確認。
- [Opus] ライセンス (GPLv3 / Royalty-free 2.0 / 商用) の扱いを確認して記録する。
- [Sonnet] シンボル棚卸しの下調べ: `private_unstable_api.rs:181-249` の re_exports、`generator/rust.rs` と `generator/cpp.rs` が参照するシンボル、インタープリターが使う `i_slint_core::*`、公開 API を一覧化する。
- [Opus] その一覧を「維持 / 継ぎ目として維持 / 非互換」に分類し、`docs/design/compat-contract.md` を確定する。
- [Opus] スパイク S1・S2・S3 の実装と結果の文書化。
- **ゲート** [Opus]: 座標系、フレーム駆動、Visual 種別、テスト方式の判断が文書化されている。

### Phase 1 — 歩く骨格 (Empty / Rectangle / Window) (Opus 中心)
- [Sonnet] コアの刈り込み (上表「捨てる」)。Windows 以外の `cfg` を除去し、ビルドを通す。削除で壊れる参照の洗い出しと修正。
- [Opus] 刈り込み後の残すべき継ぎ目 (`WindowAdapter`、`WindowInner`、`SlintContext`) の改造方針を確定し、Sonnet の刈り込み差分をレビューする。
- [Opus] `host/`: メッセージループ、`SlintContext` 直結、`invoke_from_event_loop` / `quit_event_loop`、タイマー (`update_timers_and_animations`)、HWND と `DesktopWindowTarget`、リサイズ、DPI、クローズ。
- [Opus] `visual_tree/`: ノード表、dirty キュー、同期パス。`ItemVTable::render` → `update_visual`。
- [Opus] items: Empty、Rectangle (単色)、WindowItem (最小)。以後の item 実装の雛形にする。
- [Opus] `testing`: ツリーダンプとキャプチャの仕組み。
- [Sonnet] `tests/fixtures`: 生成コードと同じ形の手書き `ItemTree` を複数用意し、ダンプのスナップショットテストを書く。
- **ゲート** [Opus]: 入れ子の色付き矩形が表示され、プロパティ変更・リサイズ・DPI 変更に追従する。ダンプ/キャプチャのテストが通る。
- **状態**: 閉じた。設定アプリでの拡大率変更には追従する (シェルによる再配置で数 px の誤差が残るが累積しない)。倍率の異なるモニター間のドラッグは、モニターがないため Phase 7 のマルチモニター検証に回す。

### Phase 2 — 構造・視覚属性・互換プローブ開始 (Opus 中心)
- [Opus] BorderRectangle (角丸、個別角丸、枠線、グラデーション) を雛形として実装する。
- [Sonnet] 雛形に倣い BasicBorderRectangle、Clip、Opacity、Layer、Transform を横展開する。
- [Opus] z 順、リピーター / 条件 / ComponentContainer の構造同期、アニメーションのティック駆動。
- [Opus] `crates/slint` ファサードの設計と、`tests/probe` で上流 `slint-build` の生成コードをビルド・実行する仕組み。
- [Sonnet] プローブ用 `.slint` サンプル (矩形・レイアウト・リピーター) の追加と、ビルドエラーの解消。
- [Opus] **item! 適合テスト**の設計: `i-slint-compiler` 1.19.0 (dev-dependency) のネイティブクラス定義と、本ランタイムの rtti を突き合わせる。
- [Sonnet] 適合テストの実装、未実装 item 一覧の管理。以後すべてのフェーズのゲートに含める。
- [Sonnet] 上流 `tests/cases` の properties / layout / models / conditional 系を本ランタイムで実行できるよう移植する。
- **ゲート** [Opus]: プローブの `.slint` が動く。移植したテストが通る。同期エンジンの設計を `docs/design/visual-tree.md` に確定する。
- **状態**: 閉じた (2026-11-17)。プローブ 3 本、上流ケース 149 件が通る。残りは `tests/driver/exclusions.rs` に理由とフェーズ付きで記載。

### Phase 3 — 入力とフォーカス (Sonnet 中心)
- [Opus] Win32 入力の受け口 (`WM_POINTER` でマウス・タッチ・ペン、ホイール、キャプチャ、カーソル) を `WindowInner::process_mouse_input` / `process_key_input` に接続する基盤と、TouchArea を雛形として実装する。
- [Sonnet] 仮想キー → `key_codes` の対応表、キーボード入力の残り、クリップボード。
- [Sonnet] FocusScope、KeyBinding、Flickable、SwipeGestureHandler、ScaleRotateGestureHandler、WindowMoveArea、Timer の横展開。
- [Sonnet] `testing`: イベント注入とモック時刻 (上流テストドライバー互換)、input / focus 系テストの移植。
- **ゲート** [Opus]: 上流 `tests/cases` の input / focus 系が通る。
- **状態**: 閉じた。input 30 件・focus 23 件が通り、残りは画像 (Phase 5)・テキスト入力 (Phase 4)・ポップアップとアクセシビリティ (Phase 6) を待つ (`tests/driver/exclusions.rs`)。マウス・タッチ・ペン・キーボード・クリップボード・ウィンドウ移動・Alt+F4 を `probe-input` で手動確認した。タッチパッドのピンチと回転は、上流の winit が Windows で送らないため対象外。2 本指のタッチによるジェスチャーは上流のコアがタッチイベントから作る。上流 `tests/cases/elements` の入力系 (Flickable、ジェスチャー、WindowMoveArea など) のドライバーへの追加は Phase 4 と並行して行う。

### Phase 4 — テキスト (Sonnet 中心、基盤は Opus)
- [Opus] スパイク S4。
- **方針の変更 (S4 の後に決定)**: レイアウトと計測は上流と同じ parley (`textlayout/sharedparley`) のまま残し、描画だけを置き換える。parley が並べたグリフを Direct2D で Composition の描画面に描く。上流の挙動 (折り返し、省略、行数制限、StyledText、選択、変換中の下線) とテストの数値がそのまま合う。アプリの挙動に不備が出たら、DirectWrite への置き換え (parley の削除) を改めて検討する。
- [Opus] テキスト描画の中核: `GlyphRenderer` を実装する描画リストの記録と、Direct2D への再生。フォントデータから DirectWrite のフォントフェイスへの変換、カラー絵文字、縁取り、スレッドごとの描画デバイスとデバイス置き換え時の再描画。
- [Opus] IME 統合 (IMM32。上流の winit と同じ) と `InputMethodRequest` の接続。ホスト側の IME の窓口は、将来 TSF に差し替えられる形にする。
- [Opus] SimpleText / ComplexText / StyledTextItem / TextInput の `update_visual` の接続 (4 つとも同じ入口なので横展開せず中核と同時に実装)。
- [Sonnet] テスト用の固定幅フォント (FixedTestFont) と `configure_test_fonts`。
- [Sonnet] TextInput (キャレット、選択、アンドゥ)、TextInputInterface、text 系テストの移植。
- **ゲート** [Opus]: 上流 `tests/cases` の text 系が通る。日本語 IME の手動チェックリストを満たす。
- **状態**: 閉じた (2026-11-18)。text 38 件が通り、残り 2 件 (インタープリター、Phase 6 の `accessible_value`) は `tests/driver/exclusions.rs` に記載。テスト用フォントにより layout・elements の待ちケースも通った。Microsoft IME で、変換のインライン表示、候補ウィンドウの位置、文節移動、確定、取り消し、パスワード欄、フォーカス移動時の確定を `probe-text` で手動確認した。変換中の文字列がフォーカスを失うときの扱いは、将来の TSF 移行に備えて TSF に合わせた (フォーカス移動で確定、非アクティブ化で保持。IMM32 では IME 自身が非アクティブ化で確定する。`docs/design/input.md`)。持ち越し: 再変換 (`IMR_RECONVERTSTRING`、TSF 移行時の候補)、文字色の円錐グラデーション (Phase 5)、角丸クリップ (Phase 5)、16384 px を超えるテキスト (仮想サーフェス、Phase 7 の性能検証で判断)。

### Phase 5 — 画像・パス・エフェクト (Sonnet 中心、基盤は Opus)
- [Opus] スパイク S5 (`docs/design/spike-s5-effects.md`)。エフェクトは Rust 実装の `IGraphicsEffectD2D1Interop` で成立し、D2D で焼く代替は不要。
- [Opus] エフェクトの基盤 (`IGraphicsEffect` の Rust 実装とファクトリーのキャッシュ)、画像 1 つにサーフェス 1 つのキャッシュ、デバイス置き換え時の再転送。ImageItem を雛形として実装する (`fit()` からスプライトの矩形とブラシ変換を作る)。
- [Sonnet] ImageItem / ClippedImage の残り: alignment、source-clip、tiling (Border エフェクト)、colorize (Composite エフェクト)、9 分割 (`CompositionNineGridBrush`)、rendering、SVG。
- [Sonnet] Path: `PathData` → D2D ジオメトリ → `CompositionPathGeometry` (fill、stroke、cap/join、fill-rule、viewbox)。
- [Sonnet] BoxShadow (`BoxShadowOptions` から D2D で焼く)、BackdropBlur (背景ブラシのぼかし)、角丸クリップ、円錐グラデーション (CPU で計算したサーフェス)。
- ウィンドウ背景素材 (Mica) は方式を確認済みだが、上流に API がないため要望があるまで実装しない。
- [Sonnet] スクリーンショットテストと基準画像の追加。
- **ゲート** [Opus]: スクリーンショットテストが基準画像と一致する。
- **状態**: 閉じた (2026-11-18)。上流のスクリーンショットケース 82 件すべてが、このフォークの基準画像 (`tests/screenshots/references`) と一致する。うち 55 件は上流の基準画像 (Skia があれば Skia) との差が規則内 (差 64 超が 1% 以下、16 超が 5% 以下)。残り 27 件はレビューで、レイアウトが同じで差がグリフのラスタライズ、フィルタリング、アンチエイリアスに限られることを確認し、理由を `tests/screenshots/exclusions.rs` の `REVIEWED` に記載した (上流の基準画像はほとんどがソフトウェアレンダラーのもので、Skia の基準画像がない)。`SLINT_SCREENSHOT_REPORT=1` で上流との差を一覧できる。テストドライバーでは、画像待ちだった 59 件が通るようになった。Phase 4 から持ち越した文字色の円錐グラデーションは実装済み。角丸クリップは、上流のテキスト描画が角丸を渡さないため不要だった (角丸の `Clip` の中のテキストは親ビジュアルのクリップで切られる)。Mica は上流に API がないため、要望があるまで実装しない。

### Phase 6 — ウィンドウ機能とプラットフォーム統合 (Sonnet 中心)
- [Opus] PopupWindow (子 HWND + 専用ターゲット、close-policy) と複数ウィンドウの設計・実装。
- [Sonnet] WindowItem の全プロパティ、Dialog、Tooltip / TooltipArea。
- [Sonnet] ContextMenu / MenuBar / Menu / MenuItem / MenuSeparator (`menus.rs` + Win32 メニュー)、SystemTrayIcon、DragArea / DropArea (OLE)、Platform グローバル。
- [Opus] アクセシビリティの統合方針 (`ItemTreeVTable` の a11y エントリから UI Automation へ、`accesskit_windows` を第一候補)。
- [Sonnet] アクセシビリティの実装と、popups / menus / accessibility 系テストの移植。
- **ゲート** [Opus]: 該当テストが通る。fluent スタイルのウィジェットギャラリーがプローブで動く。
- **状態**: 閉じた (2026-11-18)。上流の popups・accessibility・elements (Dialog、SystemTrayIcon を含む) のケースが通り、残りはテスト用バックエンドとインタープリターを待つ (Phase 7、`tests/driver/exclusions.rs`)。`tools/ui-check` で、ギャラリーのコンボボックス、Slint が描くメニューバーとサブメニュー、ツールチップ、入力欄のコンテキストメニューが上流と同じ大きさ・配置になることを確かめた (差は文字のラスタライズのみ)。ネイティブのメニューバー (ダークモードの描画を含む)、コンテキストメニュー、`probe-tray` のトレイアイコンは手動確認した。設計は `docs/design/windows.md`。アクセシビリティは AccessKit を移植し、UI Automation クライアントからのテストを `tests/input` に置いた。持ち越し: キー入力を受け取るポップアップ (コンボボックスの一覧など) がフォーカスを UI Automation に伝えること、OLE によるアプリ間のドラッグ&ドロップ (上流の winit にもないため Phase 7 の後)。

### Phase 7 — 互換性の仕上げと堅牢化 (Sonnet 中心)
- [Sonnet] `rtti` と `ffi` を全 item で有効にし、cbindgen のヘッダー生成を通す。
- [Sonnet] 上流 `tests/cases` を Rust ドライバーで全件実行し、失敗を分類して除外リストを作る。
- [Opus] 除外リストのうち設計起因のものの判断、性能 (Visual 数、メモリ、起動時間)・リーク・マルチモニター DPI の検証と対策。
  マルチモニター DPI には、Phase 1 から持ち越したモニター間ドラッグの手動確認を含める (`SLINT_FORK_TRACE_DPI` でメッセージを追える)。
- [Opus] 将来のバインディング移植 (インタープリター、Node、Python、C++) に向けたギャップ一覧を `docs/design/` に残す。
- **ゲート** [Opus]: 適合テスト 100% (`Native*` を除く)。除外リスト付きで上流テストが通る。
- **状態**: 閉じた (2026-11-18)。
  - 適合テストは 100% (`Native*` を除く)。`tools/cpp-headers` が cbindgen で C++ ヘッダー 18 本を生成し、MSVC でコンパイルを確かめた。15 本は上流と一致し、残りの差は `update_visual` と、削除した wasm・wgpu・Android の型だけ。
  - 上流 `tests/cases` の全フォルダーを実行し、1008 件が通る。残り 13 件 (インタープリター 8 件、上流のテスト用バックエンド `TestingBackend` 5 件) は設計上のギャップとして除外のまま残し、`tests/driver/exclusions.rs` に記載した。
  - 性能は `probe-perf` で測る。起動から最初の同期まで約 40 ms、ギャラリーは 106 ms・194 ビジュアル・39.5 MB。仮想化しない 5000 行 (20003 ビジュアル) の構築は 320 ms、色の変更は 17 ms、テキストの変更は 162 ms、削除は 96 ms。角丸矩形のその場更新 (5000 行の色変更が 4.2 s から 18 ms に) と、ビジュアルを新しいものから解放する修正 (2000 行の削除が 6.3 s から 37 ms に) を入れた。
  - リーク: 行・ポップアップ・ウィンドウの作成と削除の繰り返しで、メモリーとハンドルは頭打ちになる。数百個以上のテキストを、コミットを挟まずに描いて壊した場合 (1 回のディスパッチャー処理内の 2 回の同期で、`sync_now` だけが起こす) は、Composition のアトラスのページが残る。既知の制限として `docs/design/visual-tree.md` に記載した。
  - 将来のバインディングのギャップ一覧は `docs/design/bindings-gap.md`。
  - 持ち越し: マルチモニター DPI の検証 (Phase 1 から持ち越したモニター間ドラッグの手動確認を含む。モニターがないため)。仮想化しない長いリストで毎フレームテキストが変わる場合に、アトラスのページが残るかの検証。16384 px を超えるテキスト (仮想サーフェス) の判断。

## Phase 7 の後に持ち越したこと

2026-11-20 に更新を止めた記録です。開いている項目は `docs/roadmap.md` にあります。

### オーナーが Phase 7 の後に別途計画するもの
- フォーク上での自作アプリ (slokk-slint) の確認。フェーズ中の確認対象にはしない (Phase 6 で決定)。
- IMM32 から TSF への移行。IME の窓口は `crates/core/host/ime.rs` に閉じてあり、フォーカス移動時の扱いは TSF に合わせてある (`docs/design/input.md`)。再変換 (`IMR_RECONVERTSTRING`) も TSF 移行時に扱う。
- Mica などの Windows 固有の機能。上流に API がないため、方式の確認だけ済ませてある (Phase 5)。2026-11-19 に、MenuBar のない窓にはリダイレクション面を付けないようにし、背景が透明な窓で Mica が出ることを確かめた (`docs/design/windows.md` の Menus)。
- OLE によるアプリ間のドラッグ&ドロップ (`start_drag`、`IDropTarget`)。上流の winit にもない (Phase 6 で決定)。受け側 (`IDropTarget`) は 2026-11-20 に作った (`docs/design/windows.md` の「Drag and Drop」)。送り側 (`start_drag`) は残る。

### 検証の持ち越し
- マルチモニター DPI。倍率の異なるモニター間のドラッグ (Phase 1 から) と、同じモニターでの倍率の変更は、2026-11-19 に別のマシンで確かめた (論理サイズが保たれる。`docs/design/windows.md` の Window Properties)。IME とポップアップ (コンボボックスの一覧、ツールチップ、`PopupWindow`、ネイティブのコンテキストメニュー) も同じ日に確かめた。倍率の変化のあとにメニューバーの下の線が黒くなる問題 (上流でも出る) は直し、確かめた。
- 仮想化しない長いリストで毎フレームテキストが変わる場合に、アトラスのページが残るか (Phase 7、`docs/design/visual-tree.md`)。
- ポップアップのアクセシビリティを NVDA と JAWS で確かめること。キー入力を受けるポップアップのフォーカスを UI Automation に伝えること (2026-11-19) と、コンボボックスの一覧でフォーカスが選択を追うこと (2026-11-20) は、UI Automation クライアントのテストとナレーターで確かめた。ナレーターは、開くと選ばれた項目、↑↓ で各項目、閉じると元のコンボボックスを読む (`docs/design/windows.md` の Accessibility、`docs/design/windows.md` の Accessibility)。

### 未実装・判断待ち
- 16384 px を超えるテキスト (仮想サーフェス)。今は切れる (Phase 4 から)。
- Mica と、全機能のメニューバーの併用。ネイティブのメニューバーは GDI のリダイレクション面に描かれ、その面が Mica を隠す。Slint 描画のメニューバー (`SLINT_NO_MUDA`) なら Mica は出るが、Alt/F10 の入口と、メニューバーの項目の UI Automation の役割がない。候補は、ホストがコンポジションでメニューバーを描き、ドロップダウンを `TrackPopupMenuEx` で出すこと (L)。切り替え方 (環境変数か、フォーク独自の API か) も未定 (2026-11-19)。
- コミット前に描いて壊したテキストのアトラスのページ。対策の候補は、解放をコミットの完了まで遅らせること (Phase 7)。
- 将来の最適化: Slint のアニメーションの Composition アニメーションへの委譲。走査が 1 ms を超えるようなら dirty キュー方式に切り替える。ブラシを色ごとに共有するか。
- 配布の長期の形。短期は git の依存と `[patch.crates-io]` の併記で公開する (2026-11-19、`README.md`)。ライセンスの表記も決めた (フォークの変更は Copyright © Kentaro Inomata、MIT-0、`docs/design/license.md`)。別名での crates.io 公開と、SixtyFPS に確かめること (改造版への Royalty-free の適用、有償ライセンスの範囲、名前の商標) は先送り (`docs/roadmap.md`)。
- core の doctest 51 件が失敗する。doc コメントの例が、core から見えない `slint` クレートを参照しているため。`cargo test --workspace --lib --bins --tests` は通る (2026-11-19 に確認)。
- 言語バインディング (インタープリター、C++、Node、Python) の移植。順序と不足は `docs/design/bindings-gap.md`。除外中の 13 件はここに含まれる。

### 対象外とし、条件がそろえば見直すもの
- parley の DirectWrite への置き換え。アプリの挙動に不備が出たら検討する (Phase 4)。
- タッチパッドのピンチと回転。上流の winit が Windows で送らないため (Phase 3)。
- `Native*` item (Qt スタイル)、C++ から独自のプラットフォームを与える `slint-platform.h`、ビットマップフォント、`set_rendering_notifier`。

## 検証方法 (全体)

- `cargo test --workspace`: 上流から引き継ぐコアの単体テスト、item! 適合テスト、ツリーダンプのスナップショットテスト。
- キャプチャテスト: `GraphicsCaptureItem::CreateFromVisual` で取得した画像を基準 PNG と比較する (方式は S3 で確定)。
- 互換プローブ: `cargo run -p probe-<name>` で、上流コンパイラーの生成コードを本ランタイム上で起動する。
- 上流 `reference/slint-1.19.0/tests/cases` の `rust` ブロックを、本ランタイムの `testing` モジュールで実行する。
- 手動確認: リサイズ、DPI 変更 (モニター間移動)、IME、タッチ。

## 主なリスク

- **1 item = 1 Visual のコスト**: 大きなリスト表示で Visual 数が膨らむ。S2 の計測結果が悪ければ、非表示項目の Visual 省略や葉ノードのサーフェス統合を Phase 2 の設計に入れる。
- **テキスト品質**: 透過サーフェスでは ClearType が使えない。分数 DPI での鮮明さは S2 で先行確認する。
- **Win2D 非依存のエフェクト**: COM インターフェースの Rust 実装が必要。不成立なら D2D でサーフェスに焼く方式に切り替える (BackdropBlur のみ代替なし)。
- **ヘッドレス CI**: セッションに DWM がない環境ではキャプチャが動かない可能性がある。S3 で確認し、ダンプテストを主、キャプチャを従とする。
