# 運用ガイド

**リリース:** 0.2.0 · [English](operations.md) · [CLI 契約](cli.ja.md) · [規範仕様](../SPEC.ja.md)

既存の Traefik Compose project と検証用の `echo` アプリを使って説明します。Docker の状態、controller の判断根拠、実際の通信を分けて確認します。導入前に [TESTING.md](../TESTING.md) を読み、実 Engine と経路疎通の受入条件が別であることを確認してください。

## 1. 導入・更新

### 対象と配置を確認

次の配置を使います。

- `~/traefik/compose.yaml`: 既存 Traefik 構成。
- `~/traefik/compose.overwrite.yaml`: 統合した設定例。
- `~/traefik/traefik-network-controller/`: lockfile を含む本ソース一式。

`compose.overwrite.yaml` は自動読込されないため、常に両方のファイルを指定します。既存ファイルがある場合は [設定例](../examples/compose.overwrite.yaml) をマージし、無関係なサービスやネットワーク定義を上書きしません。

必要な識別情報だけを読み取ります。

```sh
docker inspect traefik --format 'id={{.Id}} name={{.Name}} project={{index .Config.Labels "com.docker.compose.project"}} service={{index .Config.Labels "com.docker.compose.service"}}'
docker version --format '{{json .Server}}'
docker compose version
```

project には既存 Traefik の値を設定します。

```sh
cd ~/traefik
export TNC_TARGET_PROJECT=your_existing_traefik_project

tnc_compose() {
  docker compose -p "$TNC_TARGET_PROJECT" \
    -f compose.yaml -f compose.overwrite.yaml "$@"
}
```

デプロイ自動化にも同じ値を保持します。`TNC_TARGET_NAME`／`TNC_TARGET_SERVICE` は観測した名前と service に合わせます。対象には設定と一致する project／service ラベルが必要で、稼働中でなければなりません。Engine 全体の `traefik_app_` 名前空間は単一の controller 管理系統で扱います。project を変えても名前空間は分かれません。

### 固定ネットワークと永続状態

既存の control／WAF と、移行中の共通ネットワークを残します。non-internal な egress を `gw_priority: 100` で追加します。controller が接続するエッジは `-1` です。Compose の通常の `priority` はデフォルトゲートウェイの設定ではありません。

`TNC_PROTECTED_NETWORKS` には固定 control／WAF／移行用ネットワークの **Docker 上の実名**を指定します。必須 egress は常に保護されます。保護された管理対象エッジは維持し、`prepare`／`retire`／`adopt` も拒否します。

初期検証は `TNC_AUTO_DETACH=false` とします。`/run/tnc` にマウントする `controller_state` ボリュームを維持し、アプリと共有したり独立した二重管理用に複製したりしません。Traefik 用 Socket Proxy は `POST="0"`、`NETWORKS="0"` を維持し、controller は独自のローカルソケットを使います。

```sh
tnc_compose config -q
tnc_compose build --pull controller
# This initial fixed-network change may recreate Traefik.
tnc_compose up -d --no-deps traefik
tnc_compose run --rm --no-deps controller --version
tnc_compose run --rm --no-deps controller capabilities
tnc_compose run --rm --no-deps controller plan
```

対象、egress、保護、ネットワークポリシーのエラーを解消してから起動します。`plan` 自体は Docker を変更しませんが、`compose run` は補助コンテナを作成・削除し、ボリュームを作る場合があります。厳密に読み取りだけを許可された作業では、稼働中の controller への `exec` または既存のローカルバイナリを使い、補助コンテナを黙って作成しません。

```sh
tnc_compose up -d --no-deps controller
tnc_compose exec -T controller traefik-network-controller doctor
tnc_compose logs --tail=100 controller
```

設定例は controller にアプリネットワークや HTTP ポートを与えず、読み取り専用 root filesystem、capability の削除、書き込み可能な状態ボリュームを使います。ただし通常の rootful Docker ソケットへのアクセスは Docker 管理権限を持ちます。rootless、user namespace remapping、特殊な所有権、Linux 以外のホストは別途検証が必要です。

### 0.1.0 からの更新

既存のラベル、名前、状態ボリュームを維持します。新しいソースをビルドし、設定検証後に controller だけを再作成します。既存接続には 0.2.0 の EndpointID に基づく所有証拠がないため、明示的な引き継ぎまでは **unowned** です。自動切断の有効化だけでは所有しません。

0.2.0 の `prepare APP` は指定アプリの作成／接続だけを行い、別アプリの同期は行いません。`plan` はバージョン付きの保存計画を出力するため、旧 JSON に依存したクライアントは更新してください。[変更履歴](../CHANGELOG.md) を参照してください。

## 2. echo アプリの追加

デプロイ前にエッジを準備します。

```sh
cd ~/traefik
tnc_compose exec -T controller traefik-network-controller prepare echo
```

`traefik_app_echo` を作成または検証し、対象 Traefik を接続します。同じ検証済み範囲で繰り返せますが、既存接続の所有権を引き継ぐ操作ではありません。一度も使用していない準備済みエッジは、明示的に廃止するまで維持します。

[examples/echo.compose.yaml](../examples/echo.compose.yaml) をアプリ用ディレクトリへ配置します。`traefik/whoami:v1.12.0` を UID／GID 65532、ポート 8080 で起動し、ホストポートを公開せず、準備済みの外部ネットワークに参加します。`ECHO_HOSTNAME` を管理下のホスト名に置き換え、既存構成の TLS resolver と WAF／認証 middleware 名を維持します。`echo.example.test` は例示名で、公的な信頼を持つ証明書は取得できません。

```sh
cd ~/echo
# Replace this placeholder with the hostname and TLS setup under test.
export ECHO_HOSTNAME=echo.example.test
docker compose config -q
docker compose up -d
```

echo はリクエストヘッダーやネットワーク情報を返します。公開した検証サービスへ秘密情報、認証 Cookie、機密性のある転送ヘッダーを送らず、検証が終わったら削除します。

構成と実際の経路を確認します。

```sh
docker network inspect traefik_app_echo --format '{{json .Containers}}'
docker inspect traefik --format '{{json .NetworkSettings.Networks}}'
# Use the actual hostname and its configured certificate trust.
curl --fail --silent --show-error "https://$ECHO_HOSTNAME/"
```

アプリが旧共通 bridge にも参加していれば、共有経路は残ります。移行時は `traefik.docker.network` の変更とアプリ側の旧接続削除を同じデプロイに含めます。全アプリの移行と疎通確認後、Traefik の旧接続削除は別途確認したインフラ変更として実施します。controller がこの移行を自動実行するわけではありません。

## 3. 計画と適用

`plan` は JSON の同期計画を出力します。所有権の保存、永続的な空状態の観測回数追加、ネットワーク作成、明示的な廃止の計画は行いません。未適用の操作を含む有効な計画でも終了コードは 0 です。

```sh
cd ~/traefik
tnc_compose exec -T controller traefik-network-controller plan > tnc-plan.json
# Review the target, policy, decisions and pending actions.
tnc_compose exec -T controller traefik-network-controller apply - < tnc-plan.json
```

計画は 300 秒で失効します。適用時に Docker を再読込し、ポリシー、識別情報、台帳の revision、許可操作を照合します。期限前でも、デーモンの通常同期で revision が進んだり、外部デプロイで状態が変わったりすると拒否されます。新しい計画を取得・確認し、JSON の書き換えや拒否の迂回はしません。

手動確認の間にデーモンが計画を無効化し続ける場合、初期状態を記録し、controller だけを一時停止して同じ状態ボリュームを使う単発操作にできます。Traefik の既存接続は残ります。

```sh
tnc_compose stop controller
tnc_compose run --rm --no-deps -T controller plan > tnc-plan.json
# Review, then apply promptly within the plan's validity window.
tnc_compose run --rm --no-deps -T controller apply - < tnc-plan.json
# Restore the daemon if it was running before this workflow.
tnc_compose up -d --no-deps controller
```

controller の停止自体が許可された作業範囲である必要があります。外部の Compose デプロイは停止されないため別に調整します。計画にはインフラ識別子とポリシーのパスが含まれるため適切に保存してください。署名付きの許可証ではありません。

## 4. 自動切断の有効化

先に [SPEC.md](../SPEC.md) の停止中参照、猶予取消、endpoint 変更、再起動、状態障害、再接続ループ防止を検証用 Engine で確認します。現状を確認します。

```sh
tnc_compose exec -T controller traefik-network-controller status
```

既存接続を所有させる場合は、引き継ぎ直前に完全な ID を取得します。

```sh
target_id=$(docker inspect traefik --format '{{.Id}}')
network_id=$(docker network inspect traefik_app_echo --format '{{.Id}}')
endpoint_id=$(docker inspect traefik --format '{{(index .NetworkSettings.Networks "traefik_app_echo").EndpointID}}')

tnc_compose exec -T controller traefik-network-controller adopt echo \
  --target-id "$target_id" --network-id "$network_id" --endpoint-id "$endpoint_id"
```

そのエッジのライフサイクルを管理させる意図がある場合だけ実施します。`adopt` は所有状態を変更し、Docker 接続そのものは変更しません。ID が変わった場合は新しい接続を確認し直し、自動的に新しい ID で再試行しません。空／欠損 ID、保護対象、ポリシー違反は拒否されます。

`TNC_AUTO_DETACH=true` と選んだ `TNC_DETACH_GRACE_SECS`（既定 300）を永続的な設定へ反映し、検証後に controller だけを再作成します。controller が自分の Compose を編集することはありません。`status` で有効なポリシーを再確認します。

| 観測 | 結果 |
| --- | --- |
| 稼働中／停止中コンテナの参照あり | 接続を維持し、切断待ちを取り消す。 |
| 準備後に一度も使用していない | 維持する。準備の放棄には明示的な廃止を使う。 |
| 使用履歴と所有権があり、参照なし | 完全な観測の下で猶予を開始／継続する。 |
| 猶予と複数回の検査を通過 | 再検証し Traefik だけを切断する。ネットワークは残す。 |
| 切断済みの空ネットワークが残る | 切断を維持し、存在だけを理由に再接続しない。 |
| アプリが復帰 | 対象 Traefik を再接続する。 |
| 所有権・一覧・endpoint・状態が不確実 | 維持または失敗として理由を報告する。 |

自動切断には異なる秒での 2 回以上の正常な観測と猶予の経過が必要です。観測の長い空白、デーモン再起動、kernel boot ID の変更（ホスト再起動後の単発操作も含む）、関連ポリシー変更では猶予を保守的にやり直します。実装は `/proc/sys/kernel/random/boot_id` を読むため、Linux の実行環境で参照できる必要があります。API 失敗を参照なしとは扱いません。`docker compose stop` ではコンテナが残るためアプリ廃止にはなりません。

## 5. アプリの廃止

対象アプリのデプロイと接続変更を止めます。アプリの既存 Compose project でコンテナを削除し、データを残す場合はボリュームを維持します。

```sh
cd ~/echo
docker compose down

cd ~/traefik
tnc_compose exec -T controller traefik-network-controller retire echo
```

データ削除を別途意図していない限り `-v` を付けません。`retire` は管理ラベル、対象、全参照を検証し、Traefik だけを強制せずに切断して接続状態を確認した後、空の管理対象ネットワークを削除します。自動切断と異なり、明示的な廃止では現在の接続が unowned でも、正しく検証された未使用エッジを削除できます。保護・識別・参照・ネットワーク属性の検査は維持します。

稼働中／停止中コンテナの参照があれば拒否します。アプリのライフサイクルを確認して対処し、`docker network disconnect --force`、prune、無関係なコンテナ削除で迂回しません。

## 6. 診断と監視

次の 3 層を分けて観測します。

| 層 | 観測内容 | 分かること |
| --- | --- | --- |
| Controller | Docker health、`healthcheck`、構造化した同期エラー | 設定対象について最近同期できたか。 |
| Docker の望ましい状態 | `status`／`plan`、実接続、所有権・保護・参照 ID | 接続がポリシーと一致するか、変更を保留する理由。 |
| 利用者の通信 | 代表ルートへの外部 HTTPS、DNS、TLS、WAF／認証 | 利用者がサービスを利用できるか。 |

0.2.0 は Prometheus endpoint や HTTP exporter を内蔵しません。ホスト側の監視ジョブで定義済み JSON と Docker health を読み、必要に応じて外部でメトリクス／ログにできます。照会に必要な読み取り権限を与え、監視だけを理由に controller をアプリネットワークへ参加させません。継続する同期失敗、古い health、必要な接続の欠落、意図しない保護／所有状態の変化、実ルート障害が主な通知候補です。猶予中や意図的な停止を自動的に障害とはしません。

```sh
tnc_compose exec -T controller traefik-network-controller status
tnc_compose exec -T controller traefik-network-controller doctor
tnc_compose exec -T controller traefik-network-controller healthcheck
tnc_compose logs --tail=100 controller
```

`doctor` は snapshot／policy とローカルの daemon health を別々に示します。`daemon_health` が `NOT_READY_OR_UNAVAILABLE` でも正常終了する場合があり、経路確認は常に `NOT_CHECKED` です。デーモン異常を終了コードで判定するには `healthcheck` を使います。health の識別には自動切断、猶予、保護ネットワーク設定も含み、異なる有効ポリシーの記録は受け付けません。

| 症状 | 最初の確認と対応 |
| --- | --- |
| Traefik の全ルートが 404 | ルーター／middleware／plugin／provider のログを確認する。ルーター欠落とバックエンド接続欠落を混同しない。 |
| 再起動後の plugin download／DNS 失敗 | 実エラー、上流 DNS の準備状況、Traefik の plugin 初期化を確認する。ネットワーク同期は無効化された plugin を再読込しない。 |
| バックエンド接続エラー | `traefik.docker.network`、完全な ID による接続、待受ポートを確認し、実経路を検証する。 |
| `OWNERSHIP_UNKNOWN` | 接続を維持し、ID 指定の明示的な引き継ぎを意図するか判断する。 |
| `STALE_PLAN` | 再観測して計画を作り直す。強制フラグは存在しない。 |
| `PROTECTED_NETWORK` | 固定ネットワークとしての意図を確認する。削除を成功させるために保護を外さない。 |
| 対象／gateway 検証失敗 | インフラ管理手順で正しい Compose 識別情報や固定 egress を設定する。 |

## 7. 状態・復旧・切り戻し

`TNC_STATE_DIR` の `attachments.json` は controller 識別情報、完全な資源／endpoint ID、ライフサイクルの証拠を保持します。同じリリース／設定と組み合わせ、原子的または静止状態のボリューム snapshot でバックアップします。台帳全体を不用意に表示したり、切断を許可するために手で編集したりしません。

状態がない場合、既存接続は unowned であり切断を許可しません。破損、不安全、互換性のない状態では失敗します。controller を止めて障害時の証拠を保存し、対応する正常なバックアップを復元するか、各接続を明示的に検証・引き継いで状態を再構築します。これは管理操作であり通常の診断コマンドではありません。台帳を複製して二重 controller にすることはできません。

自動切断を止めるには `TNC_AUTO_DETACH=false` を永続化し controller を再作成します。全同期を止めるには controller を停止し、既存接続は残します。どちらも過去の切断やネットワーク削除を自動的には戻しません。必要なエッジには `prepare APP` を使い、アプリを起動して実通信を確認します。ソースのバージョンを戻すだけでは Docker の状態は戻りません。

controller のロックは共有状態領域だけを対象にします。Docker の切断 API は期待する EndpointID を条件にできないため、外部の connect／disconnect／デプロイ操作は別途調整します。[セキュリティ上の限界](security.ja.md) と [SPEC AUTH-001／LIFE-006](../SPEC.ja.md) を参照してください。
