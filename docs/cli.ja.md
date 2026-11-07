# CLI と機械向け契約

**リリース:** 0.2.0 · **Report schema:** 1 · [English](cli.md) · [運用](operations.ja.md)

最初に実際のバイナリで利用可能なコマンドを確認します。

```sh
traefik-network-controller --version
traefik-network-controller --help
traefik-network-controller capabilities
```

この 3 つは `TNC_TARGET_PROJECT` や Docker 接続なしで実行できます。他のコマンドは対象範囲の設定が必要です。出力は既定で JSON です。`--json`、`--force`、`detach`、`prune`、暗黙のリモート Docker 切り替えはありません。

## コマンドと副作用

| 呼び出し | Docker への作用 | 永続的な所有状態への作用 |
| --- | --- | --- |
| `run` または引数なし | 起動・イベント・定期同期。接続と、有効かつ条件成立時の切断 | 観測・所有権・ライフサイクルを保存し、起動時に猶予を再設定 |
| `once` | 同じポリシーで 1 回同期 | 同じ証拠を保存。デーモンの health は更新しない |
| `plan` | 読み取りのみ。条件付き同期計画を出力 | 台帳を進めない |
| `status` | 読み取りのみ。接続・所有権・判断を出力 | 台帳を進めない |
| `doctor` | 読み取りのみ。snapshot／policy とローカル health を確認 | 台帳を進めない |
| `capabilities` | Docker アクセスなし | なし |
| `healthcheck` | Docker アクセスなし。ローカル health を確認 | なし |
| `prepare APP` | **指定したエッジだけ**を作成／検証して接続 | 確認した新規接続と準備を保存。既存接続の引き継ぎはしない |
| `retire APP` | 検証した未使用エッジから Traefik を切断しネットワークを削除 | 廃止した状態を削除。明示的な検査後は unowned の接続も廃止可能 |
| `apply PLAN_FILE_OR_DASH` | 新鮮な同期計画だけを再検証して適用 | 現在のポリシーで保存。古い／改変された前提を拒否 |
| `adopt APP --target-id ID --network-id ID --endpoint-id ID` | inspect のみ。接続／切断しない | 全検査後、指定した現在の接続の所有権を明示的に取得 |

`APP` は `[a-z0-9][a-z0-9_-]{0,47}`、ID は 64 文字の完全な小文字 hexadecimal とします。adopt の 3 つのフラグは任意の順序で 1 回ずつ指定します。他のサブコマンドに追加フラグはありません。

状態調整が必要な操作は共有 operation lock を使います。読み取りコマンドでもローカルの調整ファイルを作成・確認し、状態ディレクトリの権限を制限する場合があります。Docker や所有権台帳の変更とは別です。`compose run` によるコンテナ／ボリューム操作も別に扱います。[運用ガイド](operations.ja.md) を参照してください。

`TNC_DRY_RUN=true` は所有権を含む controller の変更を防ぎ、予定操作を報告します。run／once／prepare／retire／apply／adopt の検査は維持します。同期計画の読み取りには `plan` を使います。明示的な prepare／retire／adopt の保存計画はなく、`apply` が受け付けるのは適格な connect／disconnect を含む同期計画です。

## 環境変数

真偽値は小文字の `true` または `false` のみです。秒数は範囲内の正の 10 進整数です。

| 変数 | 既定値 | 意味／範囲 |
| --- | --- | --- |
| `TNC_TARGET_PROJECT` | 必須 | 既存 Compose project ラベルと完全一致 |
| `TNC_TARGET_NAME` | `traefik` | 正確なコンテナ名 |
| `TNC_TARGET_SERVICE` | `traefik` | 正確な Compose service ラベル |
| `TNC_EGRESS_NETWORK` | `traefik_egress` | 固定ネットワークの実名。常に保護。接続の priority は 100 |
| `TNC_SOCKET_PATH` | `/var/run/docker.sock` | 絶対パスのローカル Unix ソケット |
| `TNC_STATE_DIR` | `/run/tnc` | 絶対パスの共有永続ディレクトリ |
| `TNC_INTERVAL_SECS` | `30` | 同期間隔。1–86400 |
| `TNC_API_TIMEOUT_SECS` | `10` | Docker API の timeout。1–300 |
| `TNC_HEALTH_MAX_AGE_SECS` | `90` | daemon health の最大経過時間。1–604800 |
| `TNC_AUTO_DETACH` | `false` | 適格な所有接続の自動切断 |
| `TNC_DETACH_GRACE_SECS` | `300` | 未使用状態の猶予。1–604800 |
| `TNC_PROTECTED_NETWORKS` | 空 | 実名のカンマ区切り。前後の空白は除去し、空要素は拒否 |
| `TNC_DRY_RUN` | `false` | controller の追加変更防止条件 |

Compose 例の control／WAF／legacy 名は設定例であり、固定の組み込み名ではありません。CSV が空でも必須 egress は保護します。`attachments.json` は所有権台帳、`health.json` は別のデーモン正常性情報です。ポリシーを迂回するために編集しません。

## Report schema

[schemas/report.schema.json](../schemas/report.schema.json) が schema 1 を定義します。plan／status／once／prepare／retire／apply／adopt の出力を対象とします。doctor は report を包む別オブジェクト、capabilities は独立した能力照会です。未対応 schema／controller バージョンの保存計画は使用しません。

| フィールド | 意味 |
| --- | --- |
| `schema_version`, `controller_version` | 出力契約と作成した正確なリリース |
| `target_id` | 観測した完全な対象 ID。特定前は null |
| `dry_run` | 変更を行わない評価か |
| `managed_networks` | snapshot 内の管理対象数 |
| `observed_at`, `plan_expires_at` | Unix 秒。保存計画の有効期間は 300 秒 |
| `observation_complete` | 完全に検証された観測を取得できたか |
| `observation` | `complete`、`all_containers`、件数、Unix 秒の `started_at`／`finished_at` |
| `policy` | 対象・socket・state・egress、interval／timeout、dry run、切断／保護の有効設定 |
| `journal` | `present`、`controller_id`、`revision`。状態世代の検査用情報 |
| `inputs` | API、対象接続、ネットワーク／参照、台帳情報。鮮度検査に使う |
| `decisions` | エッジごとの状態、所有権、ライフサイクル、判断理由 |
| `pending` | この report で未適用の具体的な適格操作。予約タイマーではない |
| `applied` | 今回完了した操作 |
| `post_verified` | 必要な変更後の接続確認が完了したか。通信の正常性ではない |

decision は `network`、`network_id`、`phase`、`ownership`、`observed_attached`、`desired_attached`、nullable な `endpoint_id`、`reference_ids`、`action`、`reason_code`、nullable な `unused_since`、`empty_observations` を含みます。参照 ID には停止中の非対象コンテナも含みます。集合由来の配列は一定順序です。phase は `prepared`／`in_use`／`unused_pending`／`detached`、ownership は `unowned`／`connected`／`adopted` です。

decision の action は `connect`／`disconnect`／`keep`／`blocked` です。pending／applied の action は `operation`、`network`、nullable な `network_id`／`endpoint_id`、`reason_code` を持ちます。明示的操作は `create`、**`remove`**、`adopt` も使います。ネットワーク廃止の出力は `delete` ではなく `remove` です。

`GRACE_PENDING` ではまだ切断が適格でないため pending 操作がない場合があります。読み取り出力の `applied=[]` は変更成功の証明ではありません。正常終了でも保守的な `blocked` 判断を含む場合があります。終了コード 0 や `post_verified` だけで外部の目的がすべて達成されたと判断せず、decisions と有効なポリシーを確認します。

### 判断理由

| reason | 解釈／対応 |
| --- | --- |
| `PROTECTED_NETWORK` | 現在の接続を維持。明示的操作でも保護を迂回できない。 |
| `REFERENCED` | 他コンテナの参照があり維持する。 |
| `ATTACHMENT_MISSING` | 必要な接続が欠けている。 |
| `PREPARED_NEVER_USED` | 初回利用前の準備を維持する。 |
| `ALREADY_DETACHED` | 未使用エッジを切断状態に保つ。 |
| `AUTO_DETACH_DISABLED` | 接続専用ポリシーで維持／接続する。 |
| `OWNERSHIP_UNKNOWN` | 自動切断しない。調査し、意図する接続だけを明示的に引き継ぐ。 |
| `ENDPOINT_ID_UNAVAILABLE` | 識別証拠が切断を許可できない。 |
| `GRACE_PENDING` | 猶予と追加の完全な観測を待つ。 |
| `UNUSED_GRACE_ELAPSED` | 再検証後の自動切断が適格。 |
| `PREPARE_REQUESTED` | 明示的な準備。 |
| `RETIRE_REQUESTED` | 明示的な廃止。 |
| `ADOPT_REQUESTED` | 明示的な所有権変更。 |

## 保存計画

同じリリース／ポリシーの未改変の plan 出力を使います。`apply -` は pipe の JSON を受け取ります。ファイルパスは **プロセス／コンテナ内**を指し、ホストのファイルが自動的に見えるわけではありません。上限は 2 MiB、通常ファイルのみで symlink／特殊ファイルは拒否します。標準入力は pipe とし、10 秒以内に完了する必要があります。

apply は現在の識別情報、ポリシー、一覧、台帳 revision、許可操作を照合し、期限切れや重要な変化を拒否します。通常の daemon 同期で 300 秒以内でも revision が進む場合があります。計画は署名付きの権限ではなく、操作一覧の編集でも検査を迂回できません。[運用ガイド](operations.ja.md#3-計画と適用) の調整手順を使います。

## Doctor と health

doctor は `schema_version`、`controller_version`、`command: "doctor"`、`checks`、`report` を返します。有効な snapshot では `docker_snapshot` と `target_policy` は `PASSED`、`daemon_health` は `READY` または `NOT_READY_OR_UNAVAILABLE`、`routed_request` は `NOT_CHECKED` です。

snapshot／policy の失敗は非 0 で終了します。ローカル daemon health の欠損／未準備だけでは doctor は失敗しません。`healthcheck` は正常時に無出力で、最近の対象一致の health が欠損／未準備／古い場合は非 0 です。Docker や HTTP への通信は行いません。

daemon health を更新するのはデーモンだけです。once／status／doctor が成功しても停止・滞留中のデーモンは正常になりません。dry-run デーモンも、適格な connect／disconnect が未適用なら未準備となります。猶予中や保守的な blocked では適格な pending 操作がない場合があるため、正常性だけで外部の目的や経路の達成を証明せず、decisions と有効ポリシーも確認します。

## 終了コード・エラー・ログ

| 終了コード | 意味 |
| --- | --- |
| `0` | 契約に従って完了。pending／猶予／blocked が残る場合もある。 |
| `1` | 設定、入力、ポリシー、Docker、状態、鮮度、実行の失敗。 |
| `2` | コマンド名／引数の形が不正。 |

成功した単発操作は標準出力へ 1 つの JSON 文書を出します。help／version はテキスト、正常な healthcheck は無出力です。daemon ログと失敗は標準エラーに出すため、保存計画に混ぜません。[schemas/error.schema.json](../schemas/error.schema.json) に従い、`error.code`、長さを制限した `error.message`、利用可能なら完了済み操作の詳細を返します。生の Docker 環境変数をまとめて出す診断ではありません。

英文メッセージの断片ではなく定義された code／field で判定します。

| code | 対応 |
| --- | --- |
| `INVALID_ARGUMENT` | 記載済みの構文へ修正する。終了コード 2。 |
| `INVALID_PLAN`, `INPUT_TIMEOUT` | ファイル／標準入力を修正する。ポリシー迂回のために計画は改変しない。 |
| `STALE_PLAN` | 新しい計画を取得・確認する。拒否そのものでは台帳を進めない。 |
| `OBSERVATION_INCOMPLETE` | 完全な Docker 観測を回復する。変更コマンドでは既存の猶予継続を保守的に中断する。 |
| `JOURNAL_INVALID`, `JOURNAL_WRITE_FAILED` | 証拠を保持して状態復旧を行い、台帳を手編集して権限を作らない。 |
| `TARGET_CHANGED`, `NETWORK_CHANGED`, `OWNERSHIP_CHANGED` | 新しい世代を確認する。古い ID や所有権は自動継承しない。 |
| `PROTECTED_NETWORK`, `REFERENCES_PRESENT`, `NETWORK_NOT_MANAGED` | 設定境界と実参照を尊重し、強制 cleanup しない。 |
| `DETACH_REFUSED`, `ADOPT_REFUSED`, `ACTION_REFUSED` | 意図する操作と現在のポリシーを確認してから再試行する。 |
| `CONNECT_AMBIGUOUS`, `CONNECT_FAILED`, `DISCONNECT_AMBIGUOUS` | 実接続と完了済み操作を確認し、失敗応答だけで所有／不在を推定しない。 |
| `POST_VERIFY_FAILED`, `ENDPOINT_ID_UNAVAILABLE` | 不確実性を保って識別／接続を確認する。成功は確立していない。 |
| `OPERATION_FAILED` | 限定された診断と現状を読む。より具体的な分類がない失敗。 |

未知の code は診断情報を保持して変更を止め、迂回を推測しません。部分失敗では `error.details.applied` と現状を確認してから再試行を決めます。エラー詳細の `post_verified` は false で、非 0 だから一切の変更がなかったとは限りません。
