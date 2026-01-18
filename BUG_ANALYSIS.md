# StreamDeck StudioエミュレーターがNetworkDockとして認識される問題の分析

## 問題の概要

StreamDeck Studioのエミュレーターとして作成したものが、NetworkDockとして認識されてしまう不具合が発生しています。

## 根本原因

### クライアント側のデバイス識別ロジック

クライアント側のコード（`packages/tcp/src/hid-device/cora.ts`）では、以下のロジックでデバイスタイプを識別しています：

```typescript
// 148-153行: Promise.raceで0x80と0x08の両方に同時にリクエストを送信
const deviceInfo = await Promise.race([
    // primary port
    this.#executeSingletonCommand(0x80, true).then((data) => ({ data, isPrimary: true })),
    // secondary port
    this.#executeSingletonCommand(0x08, false).then((data) => ({ data, isPrimary: false })),
])

// 最初に応答した方でisPrimaryを決定
this.#isPrimary = deviceInfo.isPrimary

if (this.#isPrimary) {
    // 0x80のレスポンスからvendorId/productIdを読み取る（offset 12-15）
    const vendorId = dataView.getUint16(12, true)
    const productId = dataView.getUint16(14, true)
} else {
    // 0x1c（Device 2情報）を取得してvendorId/productIdを読み取る
    const rawDevice2Info = await this.#executeSingletonCommand(0x1c, true)
    const device2Info = parseDevice2Info(rawDevice2Info)
    // device2InfoからvendorId/productIdを取得
}
```

### 問題点1: Studioエミュレーターが0x08に応答している

**ファイル**: `streamdeck-cora-rs/src/bin/streamdeck-server.rs`

**問題箇所**: 1176-1188行

```rust
0x08 => {
    // Secondary port device info (Device 2 port) - Report ID 0x08
    // ...
    // For now, we return the same structure as 0x80 to acknowledge the request.
    let data = build_primary_port_info_payload(0x0fd9, 0x00aa);
    send_get_report_response(writer, message.message_id, report_id, data).await?;
    return Ok(());
}
```

**問題**: 
- Studioは**primary portデバイス**なので、0x08（secondary port）には応答すべきではありません
- 0x08に応答することで、クライアントが`isPrimary=false`と誤判断する可能性があります
- その後、0x1c（Device 2情報）を取得してデバイスタイプを判断しようとしますが、この時点で混乱が生じます

### 問題点2: NetworkDockエミュレーターが0x80に応答していない

**ファイル**: `streamdeck-cora-rs/src/bin/networkdock-server.rs`

**問題箇所**: 738-765行の`get_report_data`関数

```rust
async fn get_report_data(state: &Arc<RwLock<DeviceState>>, report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x83 => { /* Firmware version */ }
        0x84 => { /* Serial number */ }
        0x1c => { /* Device 2 info */ }
        // ❌ 0x80と0x08の処理が存在しない！
        _ => None
    }
}
```

**問題**:
- NetworkDockも**primary portデバイス**なので、0x80には応答すべきです
- 0x80に応答しないと、クライアントがデバイスを正しく識別できません
- NetworkDockのProduct IDは`0xffff`（実際のハードウェアProductIDではなく、デバイス情報クエリ時の返り値として使用される特殊な値）です

## 修正方針

### 修正1: Studioエミュレーターの0x08応答を削除

Studioエミュレーターはprimary portデバイスなので、0x08には応答しないようにする必要があります。

**修正箇所**: `streamdeck-server.rs`の1169-1189行

```rust
match report_id {
    0x80 => {
        // PRIMARY PORT: Respond with Studio device info
        log(LogLevel::Info, "[Server] Responding to GET_REPORT 0x80 (Primary Port) with Studio device info");
        let data = build_primary_port_info_payload(0x0fd9, 0x00aa);
        send_get_report_response(writer, message.message_id, report_id, data).await?;
        return Ok(());
    }
    0x08 => {
        // ❌ Studioはprimary portデバイスなので、0x08には応答しない
        // クライアントがPromise.raceで0x80と0x08の両方にリクエストを送信するが、
        // Studioは0x80のみに応答すべき
        log(LogLevel::Warn, "[Server] GET_REPORT 0x08 received, but Studio is a primary port device. Ignoring.");
        // 応答しない（Noneを返す）
        return Ok(()); // または、エラーレスポンスを返す
    }
    // ...
}
```

### 修正2: NetworkDockエミュレーターに0x80応答を追加

NetworkDockエミュレーターはprimary portデバイスなので、0x80にProduct ID `0xffff`を返す必要があります。

**修正箇所**: `networkdock-server.rs`の738-765行

```rust
async fn get_report_data(state: &Arc<RwLock<DeviceState>>, report_id: u8) -> Option<Vec<u8>> {
    match report_id {
        0x80 => {
            // Primary port device info (NetworkDock port)
            // Vendor ID: 0x0fd9 (Elgato Systems GmbH)
            // Product ID: 0xffff (NetworkDock - special value for device info query)
            // According to PROTOCOL.md: NetworkDock uses 0xffff as Product ID
            Some(build_primary_port_info_payload(0x0fd9, 0xffff))
        }
        0x83 => {
            // Firmware version
            Some(state.read().await.firmware_version.as_bytes().to_vec())
        }
        0x84 => {
            // Serial number
            Some(state.read().await.serial_number.as_bytes().to_vec())
        }
        0x1c => {
            // Device 2 (Child Device) information
            Some(build_device2_info_payload(state.read().await.device2_connected, false))
        }
        // ...
    }
}
```

また、`build_primary_port_info_payload`関数が存在しない場合は、`streamdeck-server.rs`からコピーするか、同様の関数を実装する必要があります。

## 期待される動作

修正後は以下のように動作するはずです：

1. **Studioエミュレーター**:
   - 0x80リクエストに応答 → Product ID `0x00aa`を返す
   - 0x08リクエストには応答しない
   - クライアントは0x80の応答を受け取り、`isPrimary=true`と判断
   - クライアントは0x80のレスポンスからProduct ID `0x00aa`を読み取り、Studioとして認識

2. **NetworkDockエミュレーター**:
   - 0x80リクエストに応答 → Product ID `0xffff`を返す
   - 0x08リクエストには応答しない（または応答しない）
   - クライアントは0x80の応答を受け取り、`isPrimary=true`と判断
   - クライアントは0x80のレスポンスからProduct ID `0xffff`を読み取り、NetworkDockとして認識

## 参考情報

- **Product ID一覧**（`packages/core/src/index.ts`）:
  - Studio: `0x00aa`
  - NetworkDock: `0xffff`（特殊な値）

- **PROTOCOL.md**:
  - Stream Deck Studio: `0x00aa`
  - NetworkDock: `0xffff`（実際のハードウェアProductIDではなく、デバイス情報クエリ時の返り値として使用される特殊な値）
