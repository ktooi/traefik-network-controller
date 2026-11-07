# ライセンスと再配布

正本は [English](licensing.md) です。[SPEC.md](../SPEC.md) の **PUB-001** に対応します。

## 選定結果

**このプロジェクトが独自に作成したコードとドキュメントには、MIT を採用して問題ないと判断します。** MIT は著作権表示と許諾文の保持を条件に、利用・改変・再配布・商用利用を認めます。[LICENSE](../LICENSE) を追加し、`Cargo.toml` に `license = "MIT"` を設定しました。`publish = false` は crates.io への誤った公開を防ぐ設定であり、ソースや Container の配布を禁止しません。[MIT 本文](https://opensource.org/license/mit) と [Cargo の license フィールド](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields)を参照してください。

確認した依存バージョンには、それぞれの条件と表示を維持しながら MIT の本プロジェクトと組み合わせて配布できるライセンスの選択肢があります。**MIT を設定しても、依存ライブラリや Container の OS が MIT に変わるわけではありません。** Apache-2.0 の構成要素には、そのライセンスと該当する NOTICE 等の条件が引き続き適用されます。Unicode とコード内の BSD の条件も保持します。選択したライセンスと収集した表示の全文は [THIRD-PARTY-NOTICES.txt](../THIRD-PARTY-NOTICES.txt) に含めています。

MIT は単純で、このプロジェクトの再利用を広げる方針に適しています。将来、貢献者による明示的な特許許諾と特許訴訟時の許諾終了条項が必要になれば、Apache-2.0 の第 3 条がその内容を含みます。MIT には対応する明文の条項がありません。ただし、Apache の依存ライブラリを利用するという理由だけで、本体を Apache-2.0 に変更する必要は見つかっていません。[Apache-2.0 第 3 条・第 4 条](https://www.apache.org/licenses/LICENSE-2.0)を参照してください。

著作権表示は `2026 Traefik Network Controller contributors` としています。この集合的な表記によって、権利が新しい法人等へ譲渡されることはありません。貢献者は MIT で提供する権限を持つ必要があり、外部コードを取り込む際には出典を保持します。今回の確認対象は以下のファイルと依存関係であり、全貢献者の権利関係を独立に確定する調査や特許調査ではありません。

## 確認範囲と結果

確認日: **2026-11-08 JST / 2026-11-07 UTC**。ソースの契約バージョン: **0.2.0**。

| 範囲 | 確認結果 |
| --- | --- |
| リポジトリのソース・example・プロンプト・運用スキルの配布用コピー | 確認したプロジェクトファイル内に、MIT と矛盾する既存のライセンス指定は見つかりませんでした。運用スキルの配布用コピーは、このプロジェクトの独自作成物に含まれます。 |
| `Cargo.lock` 全体と Cargo metadata | 外部パッケージは **97 個**。非 Linux 向けとビルド用の依存も含め、lockfile の全パッケージを生成した一覧で扱っています。 |
| Linux GNU の normal/build 依存グラフ | `cargo tree --locked --target x86_64-unknown-linux-gnu --edges normal,build` で数えた外部パッケージは **81 個**。ビルド用依存や crate 内の未使用部分がすべて実行ファイルへリンクされるとは限りません。 |
| crate が宣言するライセンス | MIT、Apache-2.0、Unicode-3.0 と BSL-1.0 等を含む許容的な選択肢。MIT を選べる場合は MIT、`ryu` は BSL-1.0 を選択し、`sync_wrapper` は Apache-2.0 のまま扱います。`unicode-ident` には Unicode-3.0 も必要です。 |
| 実際のライセンスファイルとソース内の表示 | **276 件の資料**を **82 種類の全文**として保持し、パッケージのバージョンと lockfile の crate checksum から参照できます。 |
| Rust 標準ライブラリ | **Rust 1.99.1** の未改変 `COPYRIGHT-library.html` を [third_party/rust/](../third_party/rust/README.md) に保持しました。`ORIGIN.json` にバージョン、compiler commit、ファイルの hash を記録しています。 |
| Debian の実行用イメージ | 各パッケージのライセンスとソース提供義務は別に残ります。この環境では最終イメージのビルド・監査は実行していません。[TESTING.md](../TESTING.md) を参照してください。 |

Cargo のライセンス文字列だけでなく、コード中の表示も確認しました。例えば `futures-channel` の `src/mpsc/queue.rs` に BSD の表示、`tracing-core` の `src/spin/` に別の著作者の MIT 表示、`icu_provider` に Rust Project の帰属表示、`tower` のオプションである Peak-EWMA 実装に Apache の帰属表示があります。特定の feature や platform がビルド対象外でも、これらを配布用表示に含めています。[BSD-2-Clause](https://spdx.org/licenses/BSD-2-Clause.html)、[Unicode-3.0](https://www.unicode.org/license.txt)、Apache-2.0 には、それぞれ該当する表示を保持する条件があります。

Rust の文書は、platform 固有のコードやビルド用資料を含む、本実行ファイルより広い上流の一覧です。記載された全構成要素が本実行ファイルへリンクされると解釈しないでください。`COPYRIGHT-library.html` の目的は Rust の [COPYRIGHT 文書](https://github.com/rust-lang/rust/blob/main/COPYRIGHT)に説明されています。このビルドを再配布する際は、同梱したリリース固有のコピーを利用してください。

reqwest 0.13.5 への更新では MIT OR Apache-2.0 の指定が維持され、Sean McArthur の MIT 著作権表示の年が 2026 に更新されています。wasm-streams 0.5.0 のライセンス本文は変更されていません。`query` feature により serde_urlencoded 0.7.1 と ryu 1.0.23 を引き続き使用し、base64 0.22.1 は lockfile から除外されました。更新した一覧は、確認済みのコード内表示と残りの全パッケージの本文を保持しています。

## 配布物に含めるもの

| 配布形態 | 保持する資料 |
| --- | --- |
| プロジェクトのソースアーカイブ | `LICENSE`、`THIRD-PARTY-NOTICES.txt`、`third_party/rust/`、`Cargo.lock`、対応するソース。 |
| イメージ外での controller バイナリ配布 | バイナリと一緒に `LICENSE`、`THIRD-PARTY-NOTICES.txt`、対応する `third_party/rust/` の表示を配布します。選択した target のシステムライブラリは別途確認します。 |
| Container イメージ | 本体・依存の表示を `/usr/share/licenses/traefik-network-controller/`、Rust の表示をその中の `rust/` に配置します。ベースイメージのパッケージの copyright ファイルと共通ライセンスも保持します。 |

OCI の `org.opencontainers.image.licenses=MIT` は controller プロジェクトのライセンスを示し、イメージ内の全ファイルが MIT であるとは意味しません。Debian は `/usr/share/doc/<package>/copyright` にパッケージのライセンスを記録し、一部の共通本文を `/usr/share/common-licenses/` に置きます。容量削減のために削除しないでください。[Debian の copyright ポリシー](https://www.debian.org/doc/debian-policy/ch-docs.html#copyright-information)と[公式 Debian イメージのライセンス案内](https://hub.docker.com/_/debian)を参照してください。

初回のイメージ公開前に、ベースの digest とパッケージ一覧を記録し、**ビルドされたイメージ**内の copyright 資料を確認し、各パッケージのライセンスが要求する対応ソースの提供方法を整えてください。上流イメージのタグや SBOM があるだけで、ソース提供義務を果たしたことにはなりません。特に Debian のベース、リンク方法、target を変更するときは、実際に配布する digest と対応付けて再確認します。Cargo の表示チェックは OS のライセンス監査を代替しません。

## 依存関係の更新

Python **3.11 以上**と固定した Rust toolchain を使います。依存を変更したら、追加・変更されたライセンス本文とコード内の表示を確認したうえで、再生成と検証を行います。

```sh
cargo fetch --locked
python3 scripts/ci/check_licenses.py --write
python3 scripts/ci/check_licenses.py
```

`--write` は `THIRD-PARTY-NOTICES.txt` のみを更新します。lockfile を変更したり、新しいライセンスを承認したりする機能ではありません。通常のチェックではファイルを変更しません。未確認のライセンス式、想定外の依存元、一覧に含まれない locked package、古い表示、Rust の表示メタデータとの不一致を拒否します。生成処理はライセンス・copyright・NOTICE のファイルと、Rust/C ソース内で条件に一致するコメントを保守的に保持しますが、汎用的なライセンス分類器でも、新しい依存が法的に適切であることの証明でもありません。差分を確認してから取り込んでください。キャッシュ済みの隔離環境では `--offline` を利用できます。

## Rust の更新

固定 toolchain を変える場合は Docker のビルダーも同じリリースに変更し、通常の検証付き Rust 配布手順で取得してください。対象 toolchain の `share/doc/rust/COPYRIGHT-library.html` を**改変せずに** `third_party/rust/` へコピーします。古いリリースでは `share/doc/rustc/` の場合があります。その文書が相対参照するライセンスファイルも含めてください。compiler 全体の `COPYRIGHT.html` や、別リリースの表示で置き換えないでください。

`third_party/rust/ORIGIN.json` を、実際の Rust リリース、完全な compiler commit、配布アーカイブの URL とパス、保持する各ファイルの SHA-256 で更新します。ディレクトリの README と本ガイドの確認情報も更新してください。チェックでは選択中の compiler のリリースと commit が出典情報に一致することと、全記録ファイルの hash を検証します。toolchain の更新は再確認を伴う変更であり、バージョン文字列の変更だけでは完了しません。

## 公開との関係

これらのファイルは MIT での配布を準備し、表示を再現可能に保つものです。リポジトリやイメージの公開操作は行いません。公開用の release tag を作成する前に、実際の CI・イメージの検証と実行用イメージの再配布条件の確認を完了してください。ライセンスの適合性、脆弱性の確認、本番運用の受入は別の検証であり、いずれか一つで他を証明できるものではありません。
