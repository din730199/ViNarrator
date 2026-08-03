# MORT — Tính năng Thuyết minh (TTS Narration) — Design Spec

- **Ngày:** 2026-06-05
- **Trạng thái:** Approved (chờ review spec)
- **Loại:** Fork & sửa thẳng MORT (C#/.NET 9 WinForms)
- **Reference source (read-only):** `.mort-ref/` (clone của https://github.com/killkimno/MORT)

## 1. Mục tiêu

Thêm khả năng **thuyết minh bằng giọng tiếng Việt** cho MORT — đọc thành tiếng nội dung game, với hai chế độ loại trừ nhau (XOR):

- **Đọc bản dịch** (ReadTranslated): OCR → dịch EN→VI → đọc bản dịch tiếng Việt.
- **Đọc bản gốc** (ReadOriginal): đọc thẳng chữ OCR được (game vốn đã tiếng Việt), **không phụ thuộc** chất lượng dịch.

Người dùng đổi qua lại hai chế độ bằng hotkey. Tại một thời điểm chỉ đọc MỘT nguồn.

## 2. Bối cảnh kỹ thuật (phát hiện từ khảo sát source)

> **MORT ĐÃ CÓ SẴN đường TTS đọc bản dịch.** Đây là việc **mở rộng/refactor**, không phải xây mới.

- TTS hiện tại: `ProcessTranslateService.DoTextToSpeach` (`.mort-ref/MORT/Service/ProcessTranslateService/ProcessTranslateService.cs:436-448`), gated bởi `SettingManager.IsUseTTS`, gọi `WindowOcr.TextToSpeach` (`.mort-ref/MORT/OcrApi/WindowOcr/WindowOcr.cs:44-77`) dùng `Windows.Media.SpeechSynthesis.SpeechSynthesizer` + `Windows.Media.Playback.MediaPlayer`.
- **TFM `net9.0-windows10.0.22621.0`** đã chiếu WinRT sẵn → **KHÔNG cần đổi `.csproj`**, không cần `Microsoft.Windows.SDK.Contracts`.
- WinRT `SpeechSynthesizer` là API **duy nhất** thấy được giọng OneCore tiếng Việt (Microsoft HoaiMy / NamMinh). `System.Speech`/SAPI5 đọc registry hive khác → **không thấy** giọng vi-VN → **tuyệt đối tránh**.
- Hotkey trong MORT: low-level `WH_KEYBOARD_LL` qua `GlobalKeyboardHook.cs` — **không** dùng `RegisterHotKey`/`WM_HOTKEY`.
- Settings: 2 store — legacy `SettingManager` (`UserData/setting.conf`) và modern khai báo `AdvencedOptionManager` (`UserData/AdvencedOptionSetting.txt`, ~3 dòng/field, tự lưu/đọc).

## 3. Quyết định thiết kế (đã chốt với user)

| # | Quyết định | Lựa chọn |
|---|-----------|----------|
| 1 | Kiến trúc | Fork & sửa thẳng MORT |
| 2 | Engine TTS | Giọng Windows OneCore (WinRT `SpeechSynthesizer`) |
| 3 | Kích hoạt | Auto-read câu mới + hotkey toggle + hotkey re-read |
| 4 | Hai chế độ | **XOR** — một chế độ tại một thời điểm, hotkey đổi |
| 5 | Ngôn ngữ đọc | Cả hai chế độ đều dùng **giọng tiếng Việt** (game gốc đã tiếng Việt) → chỉ cần chọn 1 giọng vi |
| 6 | Khi trùng câu | **Xếp hàng tuần tự** — đọc hết câu rồi đọc câu kế |
| 7 | Cap hàng đợi | **N = 3**; vượt thì bỏ câu cũ nhất (chống lag dồn) |
| 8 | Phủ engine | v1 phủ Windows OCR/Google/OneOCR/EasyOCR; **bỏ qua Tesseract** (nhánh riêng) |
| 9 | Điểm hook | **Hướng A** — gọi trực tiếp tại khối dispatch (tận dụng dedup sẵn có) |

## 4. Kiến trúc

### 4.1. Component mới: `TtsService`

- **File:** `MORT/Service/Tts/TtsService.cs` (mẫu theo `Service/TranslateTyp/TranslateTypListService.cs`).
- **Đăng ký:** DI singleton tại `Program.cs` `ConfigureServices` (~dòng 241), inject vào `ProcessTranslateService`.
- **Trách nhiệm:**
  - Giữ **1** `SpeechSynthesizer` + **1** `MediaPlayer` (`AutoPlay=false`) dùng suốt vòng đời app (không tạo mới mỗi lần đọc).
  - **Chọn giọng:** cache `SpeechSynthesizer.AllVoices` một lần; chọn giọng vi theo `VoiceInformation.Language.StartsWith("vi", OrdinalIgnoreCase)` + tên giọng đã lưu (`TtsVoiceName`). Không có giọng vi → fallback `DefaultVoice` + cờ báo "cần cài giọng vi-VN".
  - **Hàng đợi FIFO tuần tự:** `Speak(string text)` đẩy vào queue nội bộ; nghe `MediaPlayer.MediaEnded` để phát câu kế. Cap **3**: nếu queue > 3 thì bỏ câu cũ nhất.
  - **Chống đọc trùng:** bỏ qua nếu text (đã trim/normalize) trùng câu vừa enqueue/đọc gần nhất.
  - **Áp tham số:** `Options.SpeakingRate` (từ `TtsRate`), `MediaPlayer.Volume` (từ `TtsVolume`/100.0).
  - **Thread-safe & UI-free:** gọi từ thread dịch nền; serialize bằng `SemaphoreSlim`; mọi continuation/`MediaEnded` chạy ở thread nền → không động tới WinForms control (nếu cần báo UI thì `Control.BeginInvoke`).
- **API dự kiến:**
  - `void Speak(string text)` — enqueue (dedup + cap bên trong).
  - `void Stop()` — dừng + clear queue.
  - `bool HasVietnameseVoice { get; }` — để UI cảnh báo.
  - `IReadOnlyList<string> GetVietnameseVoiceNames()` — đổ vào combo chọn giọng.

### 4.2. Hai điểm hook (trong `ProcessTranslateService.DoTransAsync`)

Tại **khối dispatch đã có sẵn dedup** `if (formerOcrString.CompareTo(NowOcrString) != 0 || NowOcrString == "")` (~dòng 922, chạy đúng **1 lần/câu mới**, mọi skin):

- **Mode = ReadTranslated:** đọc `finalTransResult` (giữ logic strip `Util.GetSpliteToken` cho skin `over` ở dòng 973).
- **Mode = ReadOriginal:** đọc `NowOcrString` (text gốc OCR) thay cho bản dịch.
- Cả hai cùng đi qua một funnel (`DoTextToSpeach` repoint sang `TtsService.Speak`).
- Lưu câu vừa đọc vào field mới `_lastSpoken` (để hotkey re-read dùng lại — hiện `finalTransResult` chỉ là biến local).

> Tận dụng dedup + đúng thread của MORT; gọn hơn hook per-area trong `MakeFinalOcrAndTrans`. Bản dịch vẫn chạy ngầm & hiện overlay ở mode ReadOriginal; chỉ phần **đọc** đổi nguồn.

### 4.3. Settings (store hiện đại `AdvencedOptionManager`)

- **Tái dùng** legacy `SettingManager.IsUseTTS` làm **công tắc tổng** bật/tắt thuyết minh.
- Thêm vào `AdvencedOptionManager` (const key + field `ISettingData<T>` + `LoadTtsSetting()` gọi từ `Init()` + getter + `SetTts(...)`):
  - `TtsMode : int` (0=ReadOriginal, 1=ReadTranslated), default **1**.
  - `TtsVoiceName : string`, default `""` (rỗng = auto chọn giọng vi đầu tiên).
  - `TtsRate : int`, default 0 — thang UI **-10..+10**, map tuyến tính sang `SpeechSynthesizerOptions.SpeakingRate` (dải 0.5..2.0, 0 → 1.0).
  - `TtsVolume : int`, default 100 — dải **0..100**, map sang `MediaPlayer.Volume` (0.0..1.0).
- `enum TtsMode { ReadOriginal = 0, ReadTranslated = 1 }`.

### 4.4. Hotkey (advanced tier)

Thêm vào `KeyInputLabel.KeyType` enum, dispatch ở `Form1.gHook_KeyDown` switch (`~1236-1285`), đăng ký UI ở `UIAdvencedOption.Init` (`AddHotKey` + `CtHotKey` control + localize string):

- `ToggleTtsAutoRead` — lật `MySettingManager.IsUseTTS` (+ persist `#USE_TTS`).
- `RereadTts` — gọi `ProcessTranslateService.RereadLast()`; đọc lại `_lastSpoken` theo mode hiện tại; **chạy cả khi auto-read tắt** (hành động chủ động của user → bỏ qua gate `IsUseTTS`).
- `ToggleTtsMode` — đổi `TtsMode` giữa 0 và 1.

Tự kế thừa guard `IsLockHotKey` và "đang chọn vùng OCR" qua `AdvencedOptionManager.GetHotKeyResult`.

### 4.5. UI cấu hình (tab/group TTS trong `UIAdvencedOption`)

- CheckBox bật thuyết minh (bind `IsUseTTS`).
- ComboBox chế độ (Đọc gốc / Đọc bản dịch).
- ComboBox **chọn giọng Việt** (đổ từ `TtsService.GetVietnameseVoiceNames()`; rỗng → hiện cảnh báo "Chưa cài giọng tiếng Việt" + hướng dẫn).
- TrackBar/NumericUpDown **tốc độ** + **âm lượng**.
- 3 ô gán hotkey (`ctTtsToggle`, `ctTtsReread`, `ctTtsMode`).
- `SetTtsSetting()` gọi từ `OnClickApply` trước `AdvencedOptionManager.Save()`.

## 5. Luồng dữ liệu

```
[Capture màn hình] → [OCR engine] → NowOcrString (raw)
                                         │
                    ┌────────────────────┤
            (ReadOriginal)         (luôn) StartTrans → finalTransResult
                    │                              │
                    │            khối dispatch dedup (~922): câu MỚI?
                    │                              │
                    └──────────────┬──────────────┘
                                   ▼
                      mode? ReadOriginal→NowOcrString
                            ReadTranslated→finalTransResult
                                   ▼
                       TtsService.Speak(text)
                       (dedup → queue FIFO cap 3 → MediaEnded chain)
                                   ▼
                  SpeechSynthesizer(giọng vi) → MediaPlayer phát
```

## 6. Hướng dẫn cài giọng tiếng Việt (cho người dùng)

Nếu `AllVoices` không có entry vi: Settings → Time & Language → Language & region → Add a language → **Tiếng Việt** (đảm bảo tick **Text-to-speech**) → Speech settings tải giọng neural HoaiMy/NamMinh. Giọng chỉ xuất hiện sau khi tải xong; `TtsService` cần **tạo lại `SpeechSynthesizer`** (hoặc restart app) để thấy giọng mới.

## 7. Rủi ro & giảm thiểu

| Rủi ro | Giảm thiểu |
|--------|-----------|
| Đọc trùng mỗi frame (~2s/tick) | Hook tại khối dedup sẵn có (922) + `_lastSpoken` compare trong `TtsService` |
| Block thread dịch | `Speak` luôn async fire-and-forget; raw-read không bao giờ chờ-xong |
| 1 `MediaPlayer` dùng chung | Queue tuần tự (đúng ý user) — không tranh chấp |
| Lag dồn khi thoại nhanh | Cap queue 3, bỏ câu cũ nhất |
| Giọng vi-VN chưa cài | Detect `null` → fallback DefaultVoice + cảnh báo UI + hướng dẫn cài |
| Skin `over` có token rác | Strip `Util.GetSpliteToken` trước khi đọc (như dòng 973) |
| Cross-thread WinForms | `TtsService` UI-free; báo UI qua `Control.BeginInvoke` |
| 2 store settings lệch nhau | Enable ở legacy, phần còn lại ở modern; document rõ; cân nhắc migrate sau |
| Tesseract không được phủ | v1 chấp nhận bỏ qua (ghi rõ); bổ sung sau nếu cần |

## 8. Ngoài phạm vi (v1)

- Engine TTS đám mây (Google/Azure) hoặc AI offline (Piper/viXTTS).
- Đọc tách từng vùng OCR riêng (v1 đọc theo `NowOcrString`/`finalTransResult` gộp).
- Tự tắt hẳn bước dịch ở mode ReadOriginal để tiết kiệm API (v1 vẫn dịch ngầm cho overlay).
- Đọc các thứ tiếng khác (Nhật/Hàn/Trung) — v1 mặc định giọng Việt.

## 9. Tiêu chí hoàn thành

- Build MORT fork (.NET 9) thành công, không đổi `.csproj`.
- Bật thuyết minh → game tiếng Việt được đọc lên đúng giọng vi đã chọn.
- Đổi mode (hotkey) → chuyển giữa đọc gốc / đọc bản dịch.
- Câu mới không bị đọc trùng; thoại đọc tuần tự, không chồng tiếng.
- Re-read hotkey đọc lại câu hiện tại kể cả khi auto-read tắt.
- Chưa cài giọng vi → có cảnh báo rõ ràng, không crash.
