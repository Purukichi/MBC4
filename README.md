# MBC4 — 4-band multiband compressor (ver.0.0.9)

JUCE 8 / VST3 + Standalone。

## ビルド

```
powershell -ExecutionPolicy Bypass -File build.ps1            # Release
powershell -ExecutionPolicy Bypass -File build.ps1 -Install   # + C:\Program Files\Common Files\VST3 へコピー
```

- 必要: VS 2026 Build Tools (C++), CMake 4.x, JUCE（`../Plugin/JUCE` を参照。`-DMBC4_JUCE_DIR=` で変更可）
- パスに日本語（`P-開発`）が含まれると JUCE の juceaide が落ちるため、`build.ps1` は親フォルダを `M:` に `subst` してビルドします。
- CLAP: `libs/clap-juce-extensions` に [clap-juce-extensions](https://github.com/free-audio/clap-juce-extensions) を submodule 込みで clone すると自動で CLAP ターゲットが追加されます。

## テスト

```
cmake --build M:\Multiband-comp\build --config Release --target MBC4_Tests
M:\Multiband-comp\build\Release\MBC4_Tests.exe [snapshot.png]
```

クロスオーバー合成の平坦性・圧縮量・CPU負荷を確認し、引数があれば UI のスナップショットを書き出します。

## DSP

- クロスオーバー: Linkwitz-Riley 24 dB/oct ×3 + オールパス補正（合成フラット、レイテンシ 0）
- 各バンド: ステレオリンク・フィードフォワード、ソフトニー、Range（最大GR）、ピーク検出
- Mix: 全体 × バンドごと。ゲイン補間 = 位相ズレのないパラレルコンプ
- ゲインマッチ: Gain 行の Match ボタンで 3 秒間（無音は除外）の実効GRを平均し、補正量を Gain に一度だけ書き込む

## 操作

| グラフ | |
|---|---|
| 縦線ドラッグ | クロスオーバー（Alt+クリックで初期値） |
| 周波数タグをダブルクリック | クロスオーバーを数値入力（`1.2k` / `850` など） |
| バンド内ドラッグ | スレッショルド（Ctrl / Shift: 1/6 の微調整） |
| ダブルクリック | リセット |

ノブ: ドラッグ（Ctrl / Shift で 1/6 の微調整）、マウスオーバーで数値表示、ダブルクリックで直接入力、Alt+クリックで初期値。
略称: ths=threshold / rat=ratio / kne=knee / atk=attack / rls=release / rng=range。バンドはグラフ上で A〜D。
バンド見出しの周波数（例: `120 - 1.00k Hz`）もダブルクリックで直接入力できます。
ウィンドウ右下をドラッグすると UI 全体（文字含む）が 75〜200% で拡大縮小します。
