# プロンプト: 評価・導入

利用可能なら `traefik-network-operator` スキルを使い、Traefik Network Controller 0.2.0 を扱ってください。対応する `SPEC.md`、`docs/cli.ja.md`、`docs/operations.ja.md`、`TESTING.md` を読みます。このファイルだけで本番導入が許可されたとは扱いません。

## 入力

- 目的: `<評価 | 設定作成 | 導入>`。
- ソースリリースのパス: `<path>`。
- 既存 Traefik の Compose ディレクトリ／ファイル／project: `<値>`。
- 対象コンテナ／service: `<観測した値。通常は traefik>`。
- 固定 control／WAF／egress／旧共通ネットワークの実名: `<観測値>`。
- 許可済みの環境と変更: `<検証環境または導入範囲>`。
- 初期自動切断: 明示的な変更指示がなければ `false`。

## 指示

1. 不足情報を限定した読み取りで確認します。Docker／Compose 互換性、名前／project／service／完全な ID、固定接続／gateway priority、ソース版、永続状態、名前空間の単一管理を確認し、`.env` や環境変数全体は出力しません。
2. バイナリがあれば `--version`、`--help`、`capabilities` を確認します。JSON は既定であり、`--json`／`--force` や HTTP／MCP／metrics endpoint を作って解釈しません。
3. `examples/compose.overwrite.yaml` を基に具体的な差分を作り、既存サービス、Socket Proxy 制限、移行中の接続を保ちます。Traefik と同じ project、実名の保護対象、永続 `controller_state` を使い、ソケットの管理権限と初回 Traefik 再作成の影響を説明します。
4. 許可された環境で検証／ビルドします。`TESTING.md` を確認し、コンパイル／モックと実 Docker／通信の受入を区別します。可能かつ許可された検証を実行し、未完了は明示します。
5. 導入が許可されていれば対象範囲を適用し、読み取り plan を確認して controller を起動し、doctor／status と既存の代表ルートを別々に検証します。compose run は補助コンテナを作るため、読み取り限定では既存の実行手段を使います。
6. 必要な導入操作が権限外なら、先に確認可能な設定差分と証拠を完成させ、その未解決操作だけを確認します。既に与えられた許可は維持します。

## 完了時の証拠

対象／設定の識別、変更ファイル、有効な自動切断／保護ポリシー、実コマンド結果、導入状況、通信結果、未実施項目を示してください。切り戻しは状態や既存接続を削除せず controller の同期を停止する方法を説明します。SPEC SCOPE-001–005、AUTH-001–002、OWN-005、OPS-001–005 と AC-01/02/11/15/17/18 に対応させます。
