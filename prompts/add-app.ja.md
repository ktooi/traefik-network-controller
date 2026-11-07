# プロンプト: アプリ追加

導入済み Traefik Network Controller の契約を使ってアプリを追加してください。利用可能なら運用スキルを使い、対応する `SPEC.md`、`docs/cli.ja.md`、`docs/operations.ja.md`、`examples/echo.compose.yaml` を読みます。例は `echo`／`traefik_app_echo` であり、実作業は入力の識別子を使います。

## 入力

- アプリ識別子: `<app。検証用は echo>`。
- アプリ Compose のパス／project／予定ホスト名: `<値>`。
- 既存 Traefik／controller のパス／project: `<値>`。
- 既存の routing／TLS／WAF／認証: `<実際の名前を調べる>`。
- 許可済み変更・デプロイ: `<範囲>`。

## 指示

1. `--version`／`--help`／`capabilities`、対象識別、有効ポリシーを確認します。機密情報を除いてアプリ設定と参照を読み、可能なら稼働中 controller を記載済み Compose 手順で使います。
2. status／plan で対象エッジが正しく管理され、保護対象でなく、互換性を満たすことを確認します。同名の衝突は引き継ぎや改名の許可ではありません。拒否を弱めません。
3. 具体的なアプリ Compose 差分を作ります。Web サービスだけを外部の `traefik_app_<app>` に参加させ、`traefik.docker.network` に実名を指定します。DB／cache は内部ネットワークに残し、不要な host port や socket は付けません。意図した移行では旧共通接続も外します。volume、認証情報、承認済み image、実際の TLS／WAF／認証名を保ちます。
4. 許可された範囲で `prepare APP` の後、アプリの既存 project でデプロイします。prepare は指定エッジだけを扱い、他エッジを変更せず、既存 endpoint を所有しません。新アプリのために Traefik の中央ネットワーク一覧を編集しません。
5. 完全な ID による接続、controller 判断、backend port、実際の HTTPS を確認します。echo 例は `traefik/whoami:v1.12.0`、明示的な port8080／nonroot を使い、機密ヘッダーや環境を渡しません。
6. 結果を正確に報告します。ネットワークが正しくても経路失敗は成功とせず、保護を外す前に provider／middleware／plugin／DNS／backend を調べます。

## 完了時の証拠

変更ファイル、アプリ／project／エッジ識別、準備結果、別途確認した通信結果、残る検証を示してください。初回利用前の準備は保持されるため、デプロイを放棄した場合は範囲内で明示的廃止を提案します。SCOPE-002–005、OWN-001–002、LIFE-005、PLAN-001、DOC-005 と AC-01/02/03/08/17/18 に対応させます。
