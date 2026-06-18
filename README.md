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

## csv_plot

指定した `x` 列と `y` 列をターミナル上でプロットします。`ratatui` を使うため、このコマンドだけは `plot` feature を有効にしてビルドする必要があります。

```bash
cargo run --features plot -- csv_plot input.csv \
  --x-column stamp \
  --y-columns signal,velocity
```

プロット画面は `q`、`Esc`、`Enter` で終了します。`plot` feature なしでビルドしたバイナリでは、このコマンドは「plot support なし」と表示して実行されません。

操作:

- `d`: `dy/dx` の派生系列パネルを下段に表示/非表示
- `←` / `h`, `→` / `l`: X方向へ移動
- `↑` / `k`, `↓` / `j`: Y方向へ移動
- `+` / `=`: X/Y 同時にズームイン
- `-`: X/Y 同時にズームアウト
- `x` / `X`: X方向だけズームイン / アウト
- `y` / `Y`: Y方向だけズームイン / アウト
- `0`: 表示範囲を全体表示にリセット
