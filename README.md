# my_dev_tools

Rust で実装する開発用ツール集です。

入力CSVは通常の `.csv` に加えて `.zst` と `.tar.zst` を扱えます。`.tar.zst` / `.tar` は内部にCSVを1つだけ含む前提です。

## csv_key_diff

指定したキー列の値で 2 つの CSV の行を突き合わせ、差分を `diff` 風に標準出力します。

```bash
cargo run -- csv_key_diff left.csv right.csv --key id
cargo run -- csv_key_diff left.csv right.csv --key id --key sub_id
```

差分がなければ `no differences` を出力します。差分がある場合は以下を検出します。

- 片側にしか存在しない列名
- キー列が片側 CSV に存在しない状態
- 同じキーに対応する行の列値差分
- 片側にしか存在しない行

## csv_key_diff_extract

`csv_key_diff` が出力した差分テキストをもとに、差分のある行だけを左右それぞれ別CSVへ保存します。

```bash
cargo run -- csv_key_diff left.csv right.csv --key id --key sub_id > diff.txt
cargo run -- csv_key_diff_extract left.csv right.csv diff.txt \
  --output_left left_diff_rows.csv \
  --output_right right_diff_rows.csv
```

## csv_pseudo_diff

指定した時間列と時系列データ列から、`(現在値 - 前回値) / ((現在時刻 - 前回時刻) * time_scale)` を計算し、CSVとして標準出力します。`--output` を指定すると同じ内容をファイルにも保存します。

```bash
cargo run -- csv_pseudo_diff input.csv \
  --time-column stamp \
  --value-column signal \
  --time-scale 1.0

cargo run -- csv_pseudo_diff input.csv \
  --time-column stamp \
  --value-column signal \
  --time-scale 1.0 \
  --output pseudo_diff.csv
```

## csv_anomaly_detect

指定した `x` 列に対する局所トレンドからの乖離を見て、元CSVに異常ラベル列を追加したCSVを標準出力します。各 `y` 列について `*_anomaly_mask` 列を追加し、正常なら `0`、異常ならビットマスクを出力します。`x` 列は時系列順に単調増加している必要があり、追加されるラベル列名が既存ヘッダと衝突する場合はエラーにします。

ビット定義:

- `1`: 正方向スパイク
- `2`: 負方向スパイク
- `4`: 正方向ステップ変化
- `8`: 負方向ステップ変化

```bash
cargo run -- csv_anomaly_detect input.csv \
  --x-column stamp \
  --y-columns signal,velocity
```

## csv_label_split

指定したラベル列の値ごとに CSV を分割し、ラベル列を除いた内容を個別の CSV ファイルとして出力します。ラベルが 5 種類あれば 5 ファイル出力され、元 CSV にラベル列以外が 10 列あれば各出力 CSV も 10 列になります。

`--output-dir` を省略した場合はカレントディレクトリに出力します。出力ファイル名は `入力ファイル名_連番_ラベル名.csv` 形式で、ラベル名中のパスに使えない文字は `_` に置き換えます。

```bash
cargo run -- csv_label_split input.csv \
  --label-column label

cargo run -- csv_label_split input.csv \
  --label-column label \
  --output-dir split_csv
```

## csv_plot

指定した列をターミナル上でプロットします。プロット機能は常にビルド対象です。

```bash
cargo run -- csv_plot input.csv \
  --x-column stamp \
  --y-columns signal,velocity

cargo run -- csv_plot input.csv \
  --label-column label \
  --timestamp-column stamp \
  --value-column signal
```

状態を表すビットマスク列を `--state-column`、状態定義CSVを `--state-csv` で追加指定すると、プロット下部に状態ごとの色付き帯を表示します。状態定義CSVは `bit` と `label` が必須で、任意の `color` は `#RRGGBB` 形式です。`color` 列またはその値を省略した状態は自動配色されます。現在は最大3状態まで指定できます。複数ビットが同時に立つ場合は、それぞれの状態帯に同時表示されます。ラベル系列モードでは状態定義は全ラベルで共通ですが、状態値はラベルごとに独立して表示されます。

```csv
bit,label,color
1,positive_spike,#e15759
2,negative_spike,#4fa8dc
4,positive_step,
```

```bash
cargo run -- csv_plot input.csv \
  --x-column stamp \
  --y-columns signal \
  --state-column anomaly_mask \
  --state-csv states.csv
```

`--x-column` / `--y-columns` を指定した場合は従来通り数値列をそのままプロットします。

`--label-column` / `--timestamp-column` / `--value-column` を指定した場合は、ラベルごとに系列を自動で分割して時系列プロットします。タイムスタンプ列は Unix 秒、RFC3339、または UTC とみなせる `YYYY-MM-DD HH:MM:SS[.fraction]` / `YYYY/MM/DD HH:MM:SS[.fraction]` / `...T...` 形式を受け付けます。

プロット画面は `q`、`Esc`、`Enter` で終了します。`d` で疑似微分ペイン、`f` で FFT ペインを切り替えられ、両方同時表示もできます。FFT は現在の表示範囲だけを対象に計算し、FFT ペインが非表示の間は計算しません。

操作:

- `d`: `dy/dx` の派生系列パネルを下段に表示/非表示
- `f`: FFT パネルを下段に表示/非表示
- `←` / `h`, `→` / `l`: X方向へ移動
- `↑` / `k`, `↓` / `j`: Y方向へ移動
- `+` / `=`: X/Y 同時にズームイン
- `-`: X/Y 同時にズームアウト
- `x` / `X`: X方向だけズームイン / アウト
- `y` / `Y`: Y方向だけズームイン / アウト
- `0`: 表示範囲を全体表示にリセット

## csv_plot_image

`csv_plot` と同じ列指定で、線形プロットを画像ファイルとして保存します。出力先ディレクトリが存在しない場合は自動で作成します。画像内の文字には、サブモジュール `third_party/mplus-fonts` の M PLUS 1 Regular を埋め込むため、システムフォントは不要です。フォントのライセンスは `third_party/mplus-fonts/OFL.txt` を参照してください。

標準の出力解像度は 1600x900 です。高周波の系列には `--width` と `--height` を指定して、十分な横方向のピクセル数を確保してください。表示幅よりも点数が多い系列では、各ピクセル列の最小値と最大値を保持して描画するため、ピークが欠落しません。

```bash
cargo run -- csv_plot_image input.csv \
  --x-column stamp \
  --y-columns signal,velocity \
  --width 3840 --height 2160 \
  --output plot.png

cargo run -- csv_plot_image input.csv \
  --label-column label \
  --timestamp-column stamp \
  --value-column signal \
  --output labeled_plot.png
```

状態帯は `csv_plot` と同じ `--state-column` / `--state-csv` 指定で画像にも含められます。

```bash
cargo run -- csv_plot_image input.csv \
  --x-column stamp \
  --y-columns signal \
  --state-column anomaly_mask \
  --state-csv states.csv \
  --output plot.png
```

## csv_power_spectrum

指定した時系列 `x` 列と `y` 列から時間ごとの周波数強度を計算し、CLI 上でスペクトログラム風のヒートマップとして表示します。入力範囲全体を用いて等間隔に再サンプリングした後、時間窓ごとの FFT パワーを時間×周波数で可視化します。

```bash
cargo run -- csv_power_spectrum input.csv \
  --x-column stamp \
  --y-column signal
```
