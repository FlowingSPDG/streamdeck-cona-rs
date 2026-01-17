# TCP版Raw Panelプロトコル仕様書

## 概要

TCP版Raw Panelプロトコルは、Elgato Stream Deck Studio（およびStream Deck Plus）がネットワーク経由で制御される際に使用されるプロトコルです。USB HIDプロトコルをTCP/IP上で実装したもので、すべてのパケットは**1024バイト固定サイズ**で送受信されます。

## 接続情報

- **ポート番号**: `5343`
- **プロトコル**: TCP
- **パケットサイズ**: 1024バイト（固定）

## 1. 接続確立

### 1.1 TCP接続の開始

クライアントはStream DeckデバイスのIPアドレスに対してTCP接続を確立します。

```
接続先: <IPアドレス>:5343
```

### 1.2 シリアル番号の取得

接続確立後、クライアントはシリアル番号を取得するためにリクエストを送信します。

#### リクエスト（クライアント → デバイス）

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x03 | コマンドタイプ
1        | 0x84 | リクエストタイプ（シリアル番号取得）
2-1023   | 0x00 | パディング（ゼロ埋め）
```

**ペイロード例:**
```
03 84 00 00 00 00 ... (1024バイト)
```

#### レスポンス（デバイス → クライアント）

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x03 | コマンドタイプ
1        | 0x84 | レスポンスタイプ（シリアル番号）
2-3      | 長さ  | シリアル番号の長さ（Big Endian, 16bit）
4-(4+n)  | 文字列 | シリアル番号（ASCII文字列）
(4+n)-1023 | 0x00 | パディング
```

**ペイロード例:**
```
03 84 00 10 53 54 52 45 41 4d 44 45 43 4b 31 32 33 34 35 36 ... (1024バイト)
         ^^長さ16 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^シリアル番号
```

**実装コード:**
```169:207:comms.go
// Open a Stream Deck device (Stream Deck Studio) on TCP
func OpenTCP(IP string) (*Device, error) {

	// Server address and port
	serverAddr := IP + ":5343"

	// Connect to the TCP server in the Stream Deck
	conn, err := net.Dial("tcp", serverAddr)
	if err != nil {
		return nil, err
	}

	// Asking for serial:
	response := make([]byte, 1024)
	response[0] = 0x03
	response[1] = 0x84 // 0x81: A version number (1.00.002), 0x82: another version number (1.05.004), 0x83: another version number (1.05.008), 0x84: Serial, 0x85. Buttons asks for 84, 87, 81, 83, 86, 8a,
	_, err = conn.Write(response)
	if err != nil {
		return nil, err
	}

	serialNumber := ""

	for serialNumber == "" {
		// Reading reply for Serial:
		data := make([]byte, 1024) // 1024 for TCP, 256 was enough for USB
		n, err := conn.Read(data)
		if err != nil {
			return nil, err
		}
		// printDataAsHex(data)

		// Further more, Bitfocus Buttons sends hex sequences 03 22 01, 03 08 32 (brightness), 02 10 00 (turn off encoder light), 02 10 01 (turn off encoder light), 03 05 01, 03 05, 03 1a, 03 08 64 (brightness)

		chars := binary.BigEndian.Uint16(data[2:]) // Length:
		if data[0] == 0x03 && data[1] == 0x84 && n > int(4+chars) {
			serialNumber = string(data[4 : 4+chars])
		}
	}
```

### 1.3 その他の情報取得コマンド

コメントによると、以下のリクエストタイプも存在します：

- `0x81`: バージョン番号（1.00.002）
- `0x82`: バージョン番号（1.05.004）
- `0x83`: バージョン番号（1.05.008）
- `0x84`: シリアル番号
- `0x85`: その他の情報
- `0x87`, `0x8a`, `0x86`: その他の情報（Buttonsアプリが使用）

## 2. キープアライブ

接続維持のため、デバイスから定期的にキープアライブパケットが送信されます。

### 2.1 キープアライブ受信（デバイス → クライアント）

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x0a | キープアライブイベント
2-1023   | 0x00 | パディング
```

### 2.2 キープアライブ応答（クライアント → デバイス）

クライアントは受信後、5秒以内に応答する必要があります。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x03 | コマンドタイプ
1        | 0x1a | キープアライブ応答
2-1023   | 0x00 | パディング
```

**実装コード:**
```407:422:comms.go
				case 0xa: // Assumed "Keep alive" on the Stream Deck Studio:
					//log.Println("Received assumed keep alive from Stream Deck - sending back 03 1a reply")
					lastIncomingDataTime.Store(time.Now())

					// Prepare the response: "03 1a" followed by zeroes to make it 1024 bytes long
					response := make([]byte, 1024)
					response[0] = 0x03
					response[1] = 0x1a

					// Send the response back to the server
					_, err = d.fd.Write(response)
					if err != nil {
						log.Printf("Error sending keep alive response to server: %v\n", err)
						d.sendButtonPressEvent(-1, err)
						break
					}
```

**タイムアウト処理:**
```377:391:comms.go
	// Start a goroutine to monitor for timeout of incoming data in case we are on an IP connection:
	if d.isIP {
		go func() {
			for {
				time.Sleep(1 * time.Second) // Check every second
				// Load the last data time atomically
				lastTime := lastIncomingDataTime.Load()
				if time.Since(lastTime) > 5*time.Second {
					log.Println("No data received within 5 seconds, closing connection")
					d.fd.Close()
					return
				}
			}
		}()
	}
```

## 3. コマンド送信（クライアント → デバイス）

### 3.1 パケット構造の基本ルール

すべてのコマンドパケットは**1024バイト固定**です。ペイロードが短い場合は、残りを0x00でパディングします。

**実装コード:**
```15:26:tcp.go
func (t *TCPClient) SendFeatureReport(payload []byte) (int, error) {
	const packetSize = 1024

	// Create a buffer of 1024 bytes
	buffer := make([]byte, packetSize)

	// Copy the payload into the buffer
	copy(buffer, payload)

	// Send the entire 1024-byte buffer, regardless of payload length
	return t.conn.Write(buffer)
}
```

### 3.2 明るさ設定

ボタンの明るさを設定します（0-100%）。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x03 | コマンドタイプ
1        | 0x08 | 明るさ設定コマンド
2        | 0x00-0x64 | 明るさ値（0-100）
3-1023   | 0x00 | パディング
```

**実装コード:**
```260:273:comms.go
// SetBrightness sets the button brightness
// pct is an integer between 0-100
func (d *Device) SetBrightness(pct int) {
	if pct < 0 {
		pct = 0
	}
	if pct > 100 {
		pct = 100
	}

	preamble := d.deviceType.brightnessPacket
	payload := append(preamble, byte(pct))
	d.fd.SendFeatureReport(payload)
}
```

**brightnessPacket32の定義:**
```30:36:devices/shared.go
// brightnessPacket32 gives the brightness packet for devices which need it to be 32 bytes long
func brightnessPacket32() []byte {
	pkt := make([]byte, 2)
	pkt[0] = 0x03
	pkt[1] = 0x08
	return pkt
}
```

### 3.3 リセットコマンド

デバイスをリセットします（再起動を伴う可能性があります）。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x03 | コマンドタイプ
1        | 0x02 | リセットコマンド
2-1023   | 0x00 | パディング
```

**実装コード:**
```11:17:devices/shared.go
// resetPacket32 gives the reset packet for devices which need it to be 32 bytes long
func resetPacket32() []byte {
	pkt := make([]byte, 32)
	pkt[0] = 0x03
	pkt[1] = 0x02
	return pkt
}
```

### 3.4 ボタン画像の送信

ボタンに画像を表示します。画像は複数のパケットに分割して送信されます。

#### パケットヘッダー構造

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x02 | コマンドタイプ
1        | 0x07 | ボタン画像コマンド
2        | ボタンインデックス | ボタン番号（0-31）
3        | 0x00/0x01 | 最終パケットフラグ（0=続きあり、1=最終）
4-5      | 長さ | このパケットのペイロード長（Little Endian, 16bit）
6-7      | ページ番号 | ページ番号（Little Endian, 16bit）
8-1023   | 画像データ | JPEG画像データ + パディング
```

**実装コード:**
```16:38:devices/studio.go
// GetImageHeaderStudio returns the USB comms header for a button image for the XL
func GetImageHeaderStudio(bytesRemaining uint, btnIndex uint, pageNumber uint) []byte {
	thisLength := uint(0)
	if studioImageReportPayloadLength < bytesRemaining {
		thisLength = studioImageReportPayloadLength
	} else {
		thisLength = bytesRemaining
	}
	header := []byte{'\x02', '\x07', byte(btnIndex)}
	if thisLength == bytesRemaining {
		header = append(header, '\x01')
	} else {
		header = append(header, '\x00')
	}

	header = append(header, byte(thisLength&0xff))
	header = append(header, byte(thisLength>>8))

	header = append(header, byte(pageNumber&0xff))
	header = append(header, byte(pageNumber>>8))

	return header
}
```

**画像送信の実装:**
```676:721:comms.go
func (d *Device) rawWriteToButton(btnIndex int, rawImage []byte) error {
	// Based on set_key_image from https://github.com/abcminiuser/python-elgato-streamdeck/blob/master/src/StreamDeck/Devices/StreamDeckXL.py#L151

	if Min(Max(btnIndex, 0), int(d.deviceType.numberOfButtons)) != btnIndex {
		return errors.New(fmt.Sprintf("Invalid key index: %d", btnIndex))
	}

	pageNumber := 0
	bytesRemaining := len(rawImage)
	halfImage := len(rawImage) / 2
	bytesSent := 0

	for bytesRemaining > 0 {

		header := d.deviceType.imageHeaderFunc(uint(bytesRemaining), uint(btnIndex), uint(pageNumber))
		imageReportLength := int(d.deviceType.imagePayloadPerPage)
		imageReportHeaderLength := len(header)
		imageReportPayloadLength := imageReportLength - imageReportHeaderLength

		if halfImage > imageReportPayloadLength {
			//			log.Fatalf("image too large: %d", halfImage*2)
		}

		thisLength := 0
		if imageReportPayloadLength < bytesRemaining {
			if d.deviceType.name == "Streamdeck (original)" {
				thisLength = halfImage
			} else {
				thisLength = imageReportPayloadLength
			}
		} else {
			thisLength = bytesRemaining
		}

		payload := append(header, rawImage[bytesSent:(bytesSent+thisLength)]...)
		padding := make([]byte, imageReportLength-len(payload))

		thingToSend := append(payload, padding...)
		d.fd.Write(thingToSend)

		bytesRemaining = bytesRemaining - thisLength
		pageNumber = pageNumber + 1
		bytesSent = bytesSent + thisLength
	}
	return nil
}
```

**Stream Deck Studioの設定:**
- 画像形式: JPEG
- ボタンサイズ: 144x112ピクセル
- 1パケットあたりのペイロード: 1024バイト（ヘッダー除く）

### 3.5 エンコーダー上のディスプレイエリアへの画像送信

エンコーダーの上にあるディスプレイエリアに画像を表示します。

#### パケットヘッダー構造

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x02 | コマンドタイプ
1        | 0x0c | エリア画像コマンド
2-3      | X座標 | 開始X座標（Little Endian, 16bit）
4-5      | Y座標 | 開始Y座標（Little Endian, 16bit、通常0）
6-7      | 幅 | 画像の幅（Little Endian, 16bit）
8-9      | 高さ | 画像の高さ（Little Endian, 16bit）
10       | 0x00/0x01 | 最終パケットフラグ（0=続きあり、1=最終）
11-12    | ページ番号 | ページ番号（Little Endian, 16bit）
13-14    | 長さ | このパケットのペイロード長（Little Endian, 16bit）
15       | 0x00 | パディング
16-1023  | 画像データ | JPEG画像データ + パディング
```

**実装コード:**
```75:114:devices/studio.go
func GetImageAreaHeaderStudio(bytesRemaining uint, x, y, width, height uint, pageIndex uint) []byte {
	thisLength := uint(studioImageReportPayloadLength)
	if studioImageReportPayloadLength > bytesRemaining {
		thisLength = bytesRemaining
	}

	header := []byte{'\x02', '\x0c'}

	// 2-3 = x-start (Little Endian): 0, 200, 400, 600
	header = append(header, byte(x&0xff), byte(x>>8))

	// 4-5 = ?
	header = append(header, 0, 0)

	// 6-7 = width (Little Endian)
	header = append(header, byte(width&0xff), byte(width>>8))

	// 8-9 = height (Little Endian)
	header = append(header, byte(height&0xff), byte(height>>8))

	// 10 = 1= last, 0= before that
	if thisLength == bytesRemaining {
		header = append(header, '\x01')
	} else {
		header = append(header, '\x00')
	}

	// 11 = page index
	// 12 = ...
	header = append(header, byte(pageIndex&0xff), byte(pageIndex>>8))

	// 13-14 = (Little Endian): length (1024-16 = 1008 for full packages)
	header = append(header, byte(thisLength&0xff), byte(thisLength>>8))

	// Padding up to 16 bytes:
	header = append(header, byte(0))

	//fmt.Println(header)
	return header
}
```

### 3.6 エンコーダーのLED色設定

エンコーダーのLED色を設定します。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x02 | コマンドタイプ
1        | 0x10 | エンコーダーLED色設定
2        | エンコーダー番号 | エンコーダーインデックス（0または1）
3        | R | 赤成分（0-255）
4        | G | 緑成分（0-255）
5        | B | 青成分（0-255）
6-1023   | 0x00 | パディング
```

**実装コード:**
```298:312:comms.go
// WriteEncoderColor sets the color of an encoder
func (d *Device) WriteColorToEncoder(encIndex int, colour color.Color) {
	// Use the RGBA method to get the color components in the range of 0 to 65535.
	r16, g16, b16, _ := colour.RGBA()

	usbdata := make([]byte, 1024)
	usbdata[0] = 0x02
	usbdata[1] = 0x10
	usbdata[2] = byte(encIndex)  // Enc 0 or 1
	usbdata[3] = uint8(r16 >> 8) // R
	usbdata[4] = uint8(g16 >> 8) // G
	usbdata[5] = uint8(g16 >> 8) // B

	d.fd.Write(usbdata)
}
```

### 3.7 エンコーダーリングのLED色設定

エンコーダーリング（24個のLED）の色を設定します。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x02 | コマンドタイプ
1        | 0x0f | エンコーダーリングLED色設定
2        | エンコーダー番号 | エンコーダーインデックス（0または1）
3-74     | RGBデータ | 24個のLEDのRGB値（各3バイト、合計72バイト）
         |        | エンコーダー1の場合はオフセット0-71
         |        | エンコーダー2の場合はオフセット12-83（12要素オフセット）
75-1023  | 0x00 | パディング
```

**実装コード:**
```314:338:comms.go
// WriteEncoderColor sets the color of an encoder
func (d *Device) WriteColorToEncoderRing(encIndex int, colours []color.Color, offset uint16) {

	usbdata := make([]byte, 1024)
	usbdata[0] = 0x02
	usbdata[1] = 0x0f
	usbdata[2] = byte(encIndex) // Enc 0 or 1

	for a, colour := range colours {
		if encIndex == 1 {
			a += 12 // Right encoder ring is offset by 12 elements.
		}
		a += int(offset)
		a = a % 24

		// Use the RGBA method to get the color components in the range of 0 to 65535.
		r16, g16, b16, _ := colour.RGBA()

		usbdata[3+a*3] = uint8(r16 >> 8) // R
		usbdata[4+a*3] = uint8(g16 >> 8) // G
		usbdata[5+a*3] = uint8(b16 >> 8) // B
	}

	d.fd.Write(usbdata)
}
```

## 4. イベント受信（デバイス → クライアント）

すべてのイベントパケットは`0x01`で始まります。

### 4.1 ボタン押下イベント

標準的なボタン（Stream Deck Studioでは32個）の押下/解放イベント。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x00 | ボタンイベント（または他の値で他のイベントタイプ）
2-3      | 予約 | 予約領域
4-35     | ボタン状態 | 各ボタンの状態（1=押下、0=解放）
36-1023  | 0x00 | パディング
```

**実装コード:**
```483:501:comms.go
			} else {
				// Standard button stuff
				for i := uint(0); i < d.deviceType.numberOfButtons; i++ {
					if data[d.deviceType.buttonReadOffset+i] == 1 {
						if time.Now().After(buttonTime[i].Add(time.Duration(time.Millisecond * 100))) { // Implement 100 ms debouncing on button presses.
							if !buttonMask[i] {
								d.sendButtonPressEvent(d.mapButtonOut(i), nil)
								buttonTime[i] = time.Now()
							}
							buttonMask[i] = true
						}
					} else {
						if buttonMask[i] {
							d.sendButtonReleaseEvent(d.mapButtonOut(i), nil)
							buttonMask[i] = false // Putting it here instead of outside the condition because we ONLY want release events if there has been a Press event first (related to the fact that debouncing above can lead to ignored events)
						}
					}
				}
			}
```

### 4.2 エンコーダーイベント

エンコーダーの回転と押下イベント。

#### エンコーダー回転イベント

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x03 | エンコーダーイベント
2-3      | 予約 | 予約領域
4        | 0x01 | 回転イベント
5-8      | 回転値 | 各エンコーダーの回転値（符号付き8bit、-128〜127）
         |      | 正の値=時計回り、負の値=反時計回り
9-1023   | 0x00 | パディング
```

#### エンコーダー押下イベント

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x03 | エンコーダーイベント
2-3      | 予約 | 予約領域
4        | 0x00 | 押下イベント
5-8      | 押下状態 | 各エンコーダーの押下状態（1=押下、0=解放）
9-1023   | 0x00 | パディング
```

**実装コード:**
```451:481:comms.go
				case 3: // Encoders
					switch data[4] {
					case 1: // Rotate
						for i := 0; i < numberOfEncoders; i++ {
							if data[encoderReadOffset+i] > 0 {
								rev := int(data[encoderReadOffset+i])
								if rev > 127 {
									rev = rev - 256
								}
								d.sendEncoderRotateEvent(i, rev)
							}
						}
					case 0: // Press
						for i := 0; i < numberOfEncoders; i++ {
							if data[encoderReadOffset+i] == 1 {
								if time.Now().After(encoderButtonTime[i].Add(time.Duration(time.Millisecond * 100))) { // Implement 100 ms debouncing on button presses.
									if !encoderButtonMask[i] {
										d.sendEncoderPushEvent(i, true)
										encoderButtonTime[i] = time.Now()
									}
									encoderButtonMask[i] = true
								}
							} else {
								if encoderButtonMask[i] {
									d.sendEncoderPushEvent(i, false)
									encoderButtonMask[i] = false // Putting it here instead of outside the condition because we ONLY want release events if there has been a Press event first (related to the fact that debouncing above can lead to ignored events)
								}
							}

						}
					}
```

### 4.3 タッチイベント

タッチスクリーンのタッチイベント。

#### タップ/プレスイベント

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x02 | タッチイベント
2-3      | 予約 | 予約領域
4        | 0x01/0x02 | 0x01=タップ、0x02=プレス/ホールド
5        | 予約 | 予約領域
6-7      | X座標 | タッチ位置X（Little Endian, 16bit）
8-9      | Y座標 | タッチ位置Y（Little Endian, 16bit）
10-1023  | 0x00 | パディング
```

#### スワイプイベント

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x02 | タッチイベント
2-3      | 予約 | 予約領域
4        | 0x03 | スワイプイベント
5        | 予約 | 予約領域
6-7      | X開始 | スワイプ開始X座標（Little Endian, 16bit）
8-9      | Y開始 | スワイプ開始Y座標（Little Endian, 16bit）
10-11    | X終了 | スワイプ終了X座標（Little Endian, 16bit）
12-13    | Y終了 | スワイプ終了Y座標（Little Endian, 16bit）
14-1023  | 0x00 | パディング
```

**実装コード:**
```431:450:comms.go
				case 2: // Touch
					switch data[4] {
					case 1: // Tap
						xpos := binary.LittleEndian.Uint16(data[6:])
						ypos := binary.LittleEndian.Uint16(data[8:])
						d.sendTouchPushEvent(xpos, ypos, false)
					case 2: // Press/Hold
						xpos := binary.LittleEndian.Uint16(data[6:])
						ypos := binary.LittleEndian.Uint16(data[8:])
						d.sendTouchPushEvent(xpos, ypos, true)
					case 3: // Swipe
						xstart := binary.LittleEndian.Uint16(data[6:])
						ystart := binary.LittleEndian.Uint16(data[8:])
						xstop := binary.LittleEndian.Uint16(data[10:])
						ystop := binary.LittleEndian.Uint16(data[12:])
						/*						fmt.Printf("SWIPE: xstart=%d, ystart=%d, xstop=%d, ystop=%d; %s,%s\n", xstart, ystart, xstop, ystop,
												su.Qstr((int(xstop)-int(xstart)) > 0, "Right", "Left"), su.Qstr((int(ystop)-int(ystart)) < 0, "Up", "Down"))
						*/
						d.sendTouchSwipeEvent(xstart, ystart, xstop, ystop)
					}
```

### 4.4 NFCイベント

NFCリーダーからのデータ読み取りイベント。

```
オフセット | 値 | 説明
---------|-----|------
0        | 0x01 | イベントタイプ
1        | 0x04 | NFCイベント
2-3      | 長さ | NFCデータの長さ（Little Endian, 16bit）
4-(4+n)  | NFCデータ | NFC読み取りデータ
(4+n)-1023 | 0x00 | パディング
```

**実装コード:**
```424:430:comms.go
				case 4: // NFC
					chars := binary.LittleEndian.Uint16(data[2:])
					if len(data) > int(4+chars) {
						NFCstring := data[4 : 4+chars]
						//log.Println(NFCstring)
						d.sendNFCEvent(NFCstring)
					}
```

## 5. その他のコマンド（参考）

コメントに記載されている追加コマンド（Bitfocus Buttonsアプリが使用）：

- `0x03 0x22 0x01`: 不明
- `0x03 0x08 <値>`: 明るさ設定（既に説明済み）
- `0x02 0x10 0x00`: エンコーダーLED消灯
- `0x02 0x10 0x01`: エンコーダーLED点灯
- `0x03 0x05 0x01`: 不明
- `0x03 0x05`: 不明
- `0x03 0x1a`: キープアライブ応答（既に説明済み）

## 6. パケットサイズの重要性

すべてのパケットは**1024バイト固定**です。これはUSB HIDプロトコルの制約をTCP上で再現したものです。

- 送信時: ペイロードが短い場合は0x00でパディング
- 受信時: 常に1024バイト読み取る

**実装コード:**
```15:26:tcp.go
func (t *TCPClient) SendFeatureReport(payload []byte) (int, error) {
	const packetSize = 1024

	// Create a buffer of 1024 bytes
	buffer := make([]byte, packetSize)

	// Copy the payload into the buffer
	copy(buffer, payload)

	// Send the entire 1024-byte buffer, regardless of payload length
	return t.conn.Write(buffer)
}
```

```395:397:comms.go
		// Incoming buffer:
		data := make([]byte, 1024) // 1024 for TCP, 256 was enough for USB
		_, err := d.fd.Read(data)
```

## 7. エンディアンについて

プロトコル内で使用されるエンディアンは混在しています：

- **Big Endian**: シリアル番号取得時の長さフィールド（オフセット2-3）
- **Little Endian**: その他の数値フィールド（座標、長さ、ページ番号など）

## 8. まとめ

TCP版Raw Panelプロトコルは、USB HIDプロトコルをTCP/IP上で実装したものです。主な特徴：

1. **固定パケットサイズ**: すべてのパケットは1024バイト
2. **ポート**: 5343
3. **キープアライブ**: 5秒以内に応答が必要
4. **コマンドタイプ**: 
   - `0x01`: イベント（デバイス→クライアント）
   - `0x02`: 画像/LED制御（クライアント→デバイス）
   - `0x03`: 設定/情報取得（クライアント→デバイス）
5. **エンディアン**: Big Endian（シリアル番号長さ）とLittle Endian（その他）が混在

このプロトコルにより、Stream Deck StudioやStream Deck Plusをネットワーク経由で制御できます。
