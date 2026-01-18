# StreamDeck TCP Protocol Specification (Cora Mode)

## 概要

Stream Deck Studio/Plus は、TCP/IP経由でCoraプロトコルを使用して制御できます。

実装例:
- [node-elgato-stream-deck/tcp](https://github.com/Julusian/node-elgato-stream-deck/tree/main/packages/tcp) (TypeScript/Node.js)

USB HID実装の詳細については以下を参照：
- [productiondeck](https://github.com/FlowingSPDG/productiondeck/issues)
- [rust-streamdeck](https://github.com/ryankurte/rust-streamdeck)
- [streamdeck-rs](https://crates.io/crates/streamdeck-rs)

---

## 目次

1. [TCP 接続](#tcp-接続)
2. [Cora プロトコル](#cora-プロトコル)
3. [コマンド仕様](#コマンド仕様)
4. [イベント仕様](#イベント仕様)
5. [デバイス情報取得](#デバイス情報取得)

---

## TCP 接続

- ポート番号: `5343`（デフォルト）
- タイムアウト: 5秒（データ未受信時）

接続確立後、デバイス側が keep-alive パケットを送信。クライアントはマジックバイト `[0x43, 0x93, 0x8a, 0x41]` で検出し、ACKレスポンスを送信。

---

## Cora プロトコル

### マジックバイト

- マジックバイト: `[0x43, 0x93, 0x8a, 0x41]`

### メッセージフォーマット

```
[16バイトヘッダー][可変長ペイロード]
```

#### ヘッダー構造（16バイト）

| オフセット | サイズ | 内容 |
|-----------|--------|------|
| 0-3 | 4バイト | マジックバイト `[0x43, 0x93, 0x8a, 0x41]` |
| 4-5 | 2バイト | フラグ（Little Endian） |
| 6 | 1バイト | HIDオペレーション |
| 7 | 1バイト | 予約 |
| 8-11 | 4バイト | メッセージID（Little Endian） |
| 12-15 | 4バイト | ペイロード長（Little Endian） |

### フラグ

| フラグ | 値 | 説明 |
|--------|-----|------|
| VERBATIM | `0x8000` | セカンダリポート（子デバイス）への通信を示す。このフラグが設定されている場合、ペイロードは`[reportId]`の形式で、`0x03`プレフィックスなしで送信される |
| REQ_ACK | `0x4000` | ホストがACKを要求する |
| ACK_NAK | `0x0200` | デバイスからのACK/NAKレスポンス |
| RESULT | `0x0100` | GET_REPORT操作に対するレスポンスを示す |
| NONE | `0x0000` | フラグなし |

#### VERBATIMフラグの詳細

Coraプロトコルは、Primary（メイン）ポートとSecondary（子デバイス）ポートの2つのポートをサポートしています：

- **Primaryポート**: フラグ `NONE (0x0000)`, ペイロード `[0x03, reportId]` の形式
- **Secondaryポート**: フラグ `VERBATIM (0x8000)`, ペイロード `[reportId]` の形式（`0x03`プレフィックスなし）

この違いにより、同じTCP接続で複数のデバイス（例：NetworkDockに接続されたStream Deck Module）を管理できます。

### HID オペレーション

| オペレーション | 値 |
|---------------|-----|
| WRITE | `0x00` |
| SEND_REPORT | `0x01` |
| GET_REPORT | `0x02` |

### Keep-Alive

Keep-alive パケット（デバイス → クライアント）:
- ペイロード: `[0x01, 0x0a, ...]`
- 接続番号: ペイロードの5バイト目（インデックス5）

ACK レスポンス（クライアント → デバイス）:
- フラグ: `ACK_NAK (0x0200)`
- ペイロード: `[0x03, 0x1a, connection_no, ...]` (32バイト)

---

## コマンド仕様

### Feature Report 送信（SEND_REPORT）

```typescript
{
    flags: CoraMessageFlags.VERBATIM,
    hidOp: CoraHidOp.SEND_REPORT,
    messageId: 0,
    payload: Buffer.from(commandData),
}
```

### 明るさ設定

ペイロード:
- `[0x03, 0x08, brightness]` (brightness: 0-100)

### リセットコマンド

ペイロード:
- `[0x03, 0x02]`

### エンコーダーのLED色設定

ペイロード:
- `[0x02, 0x10, encoder_index, R, G, B]` (encoder_index: 0または1)

### エンコーダーリングのLED色設定

ペイロード:
- `[0x02, 0x0f, encoder_index, ...RGBデータ...]` (24個のLED、各3バイト、合計72バイト)

---

## イベント仕様

イベントパケットは `0x01` で始まる。

### ボタン押下イベント

ペイロード:
- `[0x01, 0x00, 0x00, 0x00, ...button_states...]` (32バイト、各ボタンの状態: 1=押下、0=解放)

### エンコーダーイベント

回転イベント:
- ペイロード: `[0x01, 0x03, 0x00, 0x00, 0x01, rot0, rot1, ...]` (符号付き8bit、正=時計回り、負=反時計回り)

押下イベント:
- ペイロード: `[0x01, 0x03, 0x00, 0x00, 0x00, press0, press1, ...]` (1=押下、0=解放)

### タッチイベント

タップ/プレス:
- ペイロード: `[0x01, 0x02, 0x00, 0x00, 0x01/0x02, 0x00, x_low, x_high, y_low, y_high]` (0x01=タップ、0x02=プレス)

スワイプ:
- ペイロード: `[0x01, 0x02, 0x00, 0x00, 0x03, 0x00, x0_low, x0_high, y0_low, y0_high, x1_low, x1_high, y1_low, y1_high]`

### NFC イベント

ペイロード:
- `[0x01, 0x04, length_low, length_high, ...nfc_data...]`

---

## デバイス情報取得

### Feature Report 取得（GET_REPORT）

プライマリポートへのリクエスト:
- フラグ: `NONE (0x0000)`
- ペイロード: `[0x03, reportId]`

セカンダリポートへのリクエスト:
- フラグ: `VERBATIM (0x8000)`
- ペイロード: `[reportId]`

### Report ID一覧

| Report ID | 内容 |
|-----------|------|
| `0x80` | プライマリポート情報取得 |
| `0x08` | セカンダリポート情報取得 |
| `0x83` | ファームウェアバージョン取得 |
| `0x84` | シリアル番号取得 |
| `0x85` | MAC アドレス取得 |
| `0x1c` | セカンダリデバイス（Device 2）情報取得 |

### シリアル番号取得

リクエスト: ペイロード `[0x03, 0x84]`

レスポンス: ペイロード `[0x03, 0x84, length_high, length_low, ...serial_string...]`
- 長さフィールドはBig Endian（他のフィールドは通常Little Endian）

---

## Studio vs NetworkDock

Stream Deck StudioとNetworkDockは、Coraプロトコルを使用する点では同一ですが、以下の違いがあります：

### 接続方式

- **Stream Deck Studio**:
  - USB HIDとTCP（Cora）の両方をサポート
  - USB接続時はUSB HIDプロトコルを使用（FeatureReport: `0x05`, `0x11`, `0x13`など）
  - TCP接続時はCoraプロトコルを使用（本ドキュメントのプロトコル）

- **NetworkDock**:
  - TCP（Cora）のみをサポート
  - USB接続は不可

### ProductID

- **Stream Deck Studio**: `0x00aa`
- **NetworkDock**: `0xffff`（実際のハードウェアProductIDではなく、デバイス情報クエリ時の返り値として使用される特殊な値）

### ハードウェア構成

- **Stream Deck Studio**:
  - 32個のボタン（16x2グリッド）
  - 2個のエンコーダー（各24段のLEDリング付き）
  - NFCリーダー
  - 2つのフルスクリーンパネル（LCD）

- **NetworkDock**:
  - 物理的なコントロールなし（`CONTROLS`が空）
  - 子デバイス（Stream Deck Module: 6-key, 15-key, 32-key）を接続可能
  - `SUPPORTS_CHILD_DEVICES: true`

### ファームウェア情報取得

ファームウェアバージョンの取得方法が異なります：

- **Stream Deck Studio (USB)**:
  - `0x05`: AP2ファームウェア
  - `0x11`: Encoder AP2ファームウェア
  - `0x13`: Encoder LDファームウェア

- **Stream Deck Studio (TCP)** / **NetworkDock (TCP)**:
  - `0x83`: AP2ファームウェアのみ

### プロトコルの互換性

内部のTCPプロトコル（Cora）自体は同一です。主な違いは：

1. **接続方法**: StudioはUSB/TCP両対応、NetworkDockはTCP専用
2. **デバイス構成**: Studioは物理コントロールあり、NetworkDockは子デバイス経由
3. **FeatureReport ID**: USB接続時のStudioとTCP接続時の違い（上記参照）

このため、同じCoraプロトコル実装で両方のデバイスをサポートできます。
