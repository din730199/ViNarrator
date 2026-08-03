# MORT — Thuyết minh (TTS Narration) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Thêm thuyết minh giọng tiếng Việt cho MORT fork với 2 chế độ XOR (đọc bản dịch / đọc text gốc), đọc tuần tự có chống trùng, cấu hình giọng/tốc độ/âm lượng + 3 hotkey.

**Architecture:** Mở rộng đường TTS sẵn có của MORT. Tách logic hàng đợi thuần (`TtsSpeechQueue`) để unit-test; gói WinRT speech vào `TtsService` (DI singleton); cắm 2 hook tại khối dispatch đã dedup của `ProcessTranslateService`; settings vào `AdvencedOptionManager`; hotkey vào advanced tier.

**Tech Stack:** C# / .NET 9 WinForms (`net9.0-windows10.0.22621.0`, x64), WinRT `Windows.Media.SpeechSynthesis` + `Windows.Media.Playback.MediaPlayer`, xUnit (test).

**Spec:** [docs/superpowers/specs/2026-06-05-mort-tts-narration-design.md](../specs/2026-06-05-mort-tts-narration-design.md)

---

## ⚠️ Quy ước repo MORT (BẮT BUỘC tuân thủ — xem `.mort-ref/CLAUDE.md`)

- **Không refactor ngoài phạm vi.** Theo style code xung quanh khi thêm tính năng.
- **Không đổi thứ tự** các enum khoá-tương-thích (`TransType`/`OcrType`/`Skin`). `KeyType` được phép **thêm member ở CUỐI** (giữ ordinal cũ).
- **Giữ format setting `@KEY ` (có dấu cách cuối)** và tên khoá serialize.
- **Không tách** các file lớn (`Form1.cs`, `SettingManager.cs`, `UIAdvencedOption.cs`, `AdvencedOptionManager.cs`...).
- **Service mới → DI** (`Program.ConfigureServices`); gọi manager cũ qua singleton `Instace` là OK.
- Logic mới → `Service/`; model → `Model/`.

> **Ghi chú đặt tên:** enum đọc gọi là **`TtsReadMode`** (KHÔNG phải `TtsMode`) để tránh trùng tên với property `AdvencedOptionManager.TtsMode` (int). Spec gọi là "TtsMode" — ở code dùng `TtsReadMode`.

---

## File Structure

**Tạo mới:**
- `MORT/Service/Tts/TtsReadMode.cs` — enum chế độ đọc (ReadOriginal/ReadTranslated).
- `MORT/Service/Tts/TtsSpeechQueue.cs` — hàng đợi thuần (dedup + cap 3), **unit-test được**.
- `MORT/Service/Tts/TtsService.cs` — DI singleton bọc WinRT speech (chọn giọng, phát tuần tự, rate/volume).
- `MORT.Tests/MORT.Tests.csproj` + `MORT.Tests/TtsSpeechQueueTests.cs` — test project (xUnit).

**Sửa:**
- `MORT/AdvencedOptionManager.cs` — thêm 4 setting TTS (mode/voice/rate/volume).
- `MORT/Service/ProcessTranslateService/ProcessTranslateService.cs` — inject `TtsService`, repoint `DoTextToSpeach`, mode branch, `RereadLast`, `_lastSpoken`.
- `MORT/Program.cs` — đăng ký DI `TtsService`.
- `MORT/Form1.cs` — inject `TtsService`, truyền vào `ProcessTranslateService`; thêm 3 case hotkey trong `gHook_KeyDown`.
- `MORT/KeyInputLabel.cs` — thêm 3 `KeyType` ở cuối enum.
- `MORT/UIAdvencedOption.cs` (+ `.Designer.cs`) — tab/group TTS: combo mode/voice, trackbar rate/volume, 3 ô hotkey.

**KHÔNG đụng:** `WindowOcr.cs` (đường TTS cũ để nguyên, không xoá).

---

## Task 0: Chuẩn bị working copy + branch + build baseline

**Files:** (không sửa code) — thiết lập môi trường.

- [ ] **Step 1: Tạo working copy của fork** (giữ `.mort-ref/` làm tham khảo read-only)

```powershell
# Từ c:\D\goslynk-vide. Khuyến nghị fork killkimno/MORT trên GitHub trước rồi clone fork của bạn.
# Local-only cũng được (clone upstream):
git clone https://github.com/killkimno/MORT.git MORT
cd MORT
git checkout -b feature/tts-narration
```

- [ ] **Step 2: Kiểm tra TFM & build C# (compile)**

Run: `dotnet build MORT/MORT.csproj -c Debug`
Expected: BUILD SUCCEEDED. (Chạy app cần DLL native `MORT_CORE.DLL`/`nhocr.DLL` từ bản release — chỉ cần để **chạy thử cuối**, không cần để compile.)

> Nếu `dotnet build` lỗi do thiếu native ref: build bằng Visual Studio 2022 (x64) như README MORT hướng dẫn. Compile được là đủ cho các task code.

- [ ] **Step 3: Commit baseline branch**

```powershell
git commit --allow-empty -m "chore: start feature/tts-narration"
```

---

## Task 1: `TtsReadMode` enum + `TtsSpeechQueue` (TDD)

**Files:**
- Create: `MORT/Service/Tts/TtsReadMode.cs`
- Create: `MORT/Service/Tts/TtsSpeechQueue.cs`
- Create: `MORT.Tests/MORT.Tests.csproj`
- Test: `MORT.Tests/TtsSpeechQueueTests.cs`

- [ ] **Step 1: Tạo enum `TtsReadMode`**

`MORT/Service/Tts/TtsReadMode.cs`:

```csharp
namespace MORT.Service.Tts
{
    /// <summary>Chế độ đọc thuyết minh (lưu dưới int trong AdvencedOptionManager.TtsMode).</summary>
    public enum TtsReadMode
    {
        ReadOriginal = 0,   // đọc text gốc OCR (không qua dịch)
        ReadTranslated = 1, // đọc bản dịch tiếng Việt
    }
}
```

- [ ] **Step 2: Tạo test project**

`MORT.Tests/MORT.Tests.csproj`:

```xml
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <TargetFramework>net9.0-windows10.0.22621.0</TargetFramework>
    <Platforms>x64</Platforms>
    <Nullable>disable</Nullable>
    <IsPackable>false</IsPackable>
    <UseWindowsForms>true</UseWindowsForms>
  </PropertyGroup>
  <ItemGroup>
    <PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.11.1" />
    <PackageReference Include="xunit" Version="2.9.2" />
    <PackageReference Include="xunit.runner.visualstudio" Version="2.8.2" />
  </ItemGroup>
  <ItemGroup>
    <ProjectReference Include="..\MORT\MORT.csproj" />
  </ItemGroup>
</Project>
```

Thêm vào solution:

```powershell
dotnet sln MORT.sln add MORT.Tests/MORT.Tests.csproj
```

> **Contingency:** nếu reference MORT (WinExe) gây lỗi build test, tách `TtsSpeechQueue` sang class library `MORT.Tts.Core` rồi cho cả MORT lẫn test tham chiếu. Mặc định thử cách reference thẳng trước.

- [ ] **Step 3: Viết test thất bại trước**

`MORT.Tests/TtsSpeechQueueTests.cs`:

```csharp
using MORT.Service.Tts;
using Xunit;

namespace MORT.Tests
{
    public class TtsSpeechQueueTests
    {
        [Fact]
        public void Enqueue_Empty_Rejected()
        {
            var q = new TtsSpeechQueue();
            Assert.False(q.Enqueue("   "));
            Assert.Equal(0, q.Count);
        }

        [Fact]
        public void Enqueue_NewText_Accepted()
        {
            var q = new TtsSpeechQueue();
            Assert.True(q.Enqueue("xin chao"));
            Assert.Equal(1, q.Count);
        }

        [Fact]
        public void Enqueue_DuplicateConsecutive_RejectedEvenAfterDequeue()
        {
            var q = new TtsSpeechQueue();
            Assert.True(q.Enqueue("a"));
            Assert.True(q.TryDequeue(out _));
            Assert.False(q.Enqueue("a")); // dedup giữ qua cả dequeue -> không đọc lại OCR cũ mỗi frame
        }

        [Fact]
        public void Enqueue_TrimmedDuplicate_Rejected()
        {
            var q = new TtsSpeechQueue();
            Assert.True(q.Enqueue(" a "));
            Assert.False(q.Enqueue("a"));
        }

        [Fact]
        public void Enqueue_OverCap_DropsOldest()
        {
            var q = new TtsSpeechQueue();
            q.Enqueue("a"); q.Enqueue("b"); q.Enqueue("c"); q.Enqueue("d");
            Assert.Equal(3, q.Count);
            Assert.True(q.TryDequeue(out var first));
            Assert.Equal("b", first); // "a" bị bỏ
        }

        [Fact]
        public void ResetDedup_AllowsReSpeak()
        {
            var q = new TtsSpeechQueue();
            q.Enqueue("a");
            q.TryDequeue(out _);
            q.ResetDedup();
            Assert.True(q.Enqueue("a")); // re-read chủ động
        }
    }
}
```

- [ ] **Step 4: Chạy test để xác nhận FAIL**

Run: `dotnet test MORT.Tests/MORT.Tests.csproj`
Expected: FAIL (compile error — `TtsSpeechQueue` chưa tồn tại).

- [ ] **Step 5: Cài đặt `TtsSpeechQueue`**

`MORT/Service/Tts/TtsSpeechQueue.cs`:

```csharp
using System.Collections.Generic;

namespace MORT.Service.Tts
{
    /// <summary>
    /// Hàng đợi đọc TTS thuần logic (không phụ thuộc WinRT) — dễ unit test.
    /// - Chống đọc trùng: bỏ qua câu trùng câu vừa nhận gần nhất (sau Trim), giữ qua cả dequeue.
    /// - Cap: giữ tối đa MaxSize câu; vượt thì bỏ câu cũ nhất.
    /// </summary>
    public sealed class TtsSpeechQueue
    {
        public const int MaxSize = 3;

        private readonly Queue<string> _queue = new Queue<string>();
        private readonly object _lock = new object();
        private string _lastAccepted = "";

        /// <summary>Đẩy câu vào hàng đợi. true nếu được nhận (không rỗng, không trùng).</summary>
        public bool Enqueue(string text)
        {
            string normalized = (text ?? "").Trim();
            if (normalized.Length == 0)
            {
                return false;
            }

            lock (_lock)
            {
                if (normalized == _lastAccepted)
                {
                    return false;
                }

                _lastAccepted = normalized;
                _queue.Enqueue(normalized);

                while (_queue.Count > MaxSize)
                {
                    _queue.Dequeue();
                }

                return true;
            }
        }

        public bool TryDequeue(out string text)
        {
            lock (_lock)
            {
                if (_queue.Count > 0)
                {
                    text = _queue.Dequeue();
                    return true;
                }
            }

            text = "";
            return false;
        }

        public int Count
        {
            get { lock (_lock) { return _queue.Count; } }
        }

        public void Clear()
        {
            lock (_lock)
            {
                _queue.Clear();
            }
        }

        /// <summary>Reset bộ nhớ chống-trùng (dùng cho re-read chủ động).</summary>
        public void ResetDedup()
        {
            lock (_lock)
            {
                _lastAccepted = "";
            }
        }
    }
}
```

- [ ] **Step 6: Chạy test để xác nhận PASS**

Run: `dotnet test MORT.Tests/MORT.Tests.csproj`
Expected: PASS (6/6).

- [ ] **Step 7: Commit**

```powershell
git add MORT/Service/Tts/TtsReadMode.cs MORT/Service/Tts/TtsSpeechQueue.cs MORT.Tests/ MORT.sln
git commit -m "feat(tts): add TtsReadMode enum and tested TtsSpeechQueue"
```

---

## Task 2: `TtsService` (WinRT speech, phát tuần tự)

**Files:**
- Create: `MORT/Service/Tts/TtsService.cs`

- [ ] **Step 1: Cài đặt `TtsService`**

`MORT/Service/Tts/TtsService.cs`:

```csharp
using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Windows.Media.Core;
using Windows.Media.Playback;
using Windows.Media.SpeechSynthesis;

namespace MORT.Service.Tts
{
    /// <summary>
    /// Dịch vụ thuyết minh: bọc WinRT SpeechSynthesizer + MediaPlayer.
    /// Phát TUẦN TỰ qua hàng đợi (đọc hết câu rồi đọc câu kế), chống trùng, áp giọng/tốc độ/âm lượng.
    /// UI-free, an toàn gọi từ thread dịch nền.
    /// </summary>
    public sealed class TtsService
    {
        private readonly SpeechSynthesizer _synth = new SpeechSynthesizer();
        private readonly MediaPlayer _player = new MediaPlayer { AutoPlay = false };
        private readonly TtsSpeechQueue _queue = new TtsSpeechQueue();
        private readonly object _pumpLock = new object();
        private bool _isPlaying = false;
        private List<VoiceInformation> _voices;
        private MediaSource _currentSource;

        public TtsService()
        {
            _voices = SpeechSynthesizer.AllVoices.ToList();
            _player.MediaEnded += OnMediaEnded;
            _player.MediaFailed += OnMediaFailed;
        }

        public bool HasVietnameseVoice =>
            _voices.Any(v => v.Language.StartsWith("vi", StringComparison.OrdinalIgnoreCase));

        public IReadOnlyList<string> GetVietnameseVoiceNames() =>
            _voices.Where(v => v.Language.StartsWith("vi", StringComparison.OrdinalIgnoreCase))
                   .Select(v => v.DisplayName).ToList();

        /// <summary>Đọc lại danh sách giọng (gọi sau khi user vừa cài thêm giọng vi-VN).</summary>
        public void RefreshVoices() => _voices = SpeechSynthesizer.AllVoices.ToList();

        /// <summary>Đẩy câu vào hàng đợi đọc (dedup + cap bên trong). Không block.</summary>
        public void Speak(string text)
        {
            if (_queue.Enqueue(text))
            {
                Pump();
            }
        }

        /// <summary>Đọc ngay 1 câu, bỏ qua dedup (dùng cho hotkey re-read).</summary>
        public void SpeakNow(string text)
        {
            _queue.ResetDedup();
            Speak(text);
        }

        public void Stop()
        {
            _queue.Clear();
            lock (_pumpLock) { _isPlaying = false; }
            try { _player.Pause(); } catch { /* ignore */ }
        }

        private void Pump()
        {
            string next;
            lock (_pumpLock)
            {
                if (_isPlaying) return;
                if (!_queue.TryDequeue(out next)) return;
                _isPlaying = true;
            }
            _ = SpeakInternalAsync(next);
        }

        private async Task SpeakInternalAsync(string text)
        {
            try
            {
                _synth.Voice = PickVoice();
                ApplyRate();

                SpeechSynthesisStream stream = await _synth.SynthesizeTextToStreamAsync(text);

                _currentSource?.Dispose();
                _currentSource = MediaSource.CreateFromStream(stream, stream.ContentType);
                _player.Source = _currentSource;
                _player.Volume = ClampVolume(AdvencedOptionManager.TtsVolume) / 100.0;
                _player.Play();
            }
            catch
            {
                // Tổng hợp/đọc lỗi -> bỏ câu này, chạy tiếp câu kế.
                lock (_pumpLock) { _isPlaying = false; }
                Pump();
            }
        }

        private void OnMediaEnded(MediaPlayer sender, object args)
        {
            lock (_pumpLock) { _isPlaying = false; }
            Pump();
        }

        private void OnMediaFailed(MediaPlayer sender, MediaPlayerFailedEventArgs args)
        {
            lock (_pumpLock) { _isPlaying = false; }
            Pump();
        }

        private VoiceInformation PickVoice()
        {
            string saved = AdvencedOptionManager.TtsVoiceName;
            VoiceInformation chosen = null;

            if (!string.IsNullOrEmpty(saved))
            {
                chosen = _voices.FirstOrDefault(v => v.DisplayName == saved);
            }

            chosen ??= _voices.FirstOrDefault(
                v => v.Language.StartsWith("vi", StringComparison.OrdinalIgnoreCase));

            return chosen ?? _synth.Voice; // fallback: giữ DefaultVoice hiện tại
        }

        private void ApplyRate()
        {
            // TtsRate -10..+10 -> SpeakingRate 0.5..2.0 (0 -> 1.0)
            int rate = Math.Max(-10, Math.Min(10, AdvencedOptionManager.TtsRate));
            double speaking = rate >= 0
                ? 1.0 + (rate / 10.0) * 1.0
                : 1.0 + (rate / 10.0) * 0.5;
            _synth.Options.SpeakingRate = speaking;
        }

        private static int ClampVolume(int v) => Math.Max(0, Math.Min(100, v));
    }
}
```

> Lưu ý: `AdvencedOptionManager.TtsVolume/TtsRate/TtsVoiceName` tạo ở Task 3. Task này build cùng Task 3 hoặc tạm dùng hằng số rồi nối lại — khuyến nghị làm Task 3 ngay sau, build chung.

- [ ] **Step 2: Verify compile** (sau khi Task 3 xong)

Run: `dotnet build MORT/MORT.csproj -c Debug`
Expected: BUILD SUCCEEDED.

- [ ] **Step 3: Smoke test thủ công (tuỳ chọn, gỡ sau)**

Tạm thêm vào cuối `Form1` constructor (sau khi DI có `_ttsService`, tức sau Task 4): `_ttsService.Speak("Xin chào, đây là thử giọng thuyết minh.");` → chạy app → nghe có đọc không → **gỡ dòng này** trước khi commit Task 4.

- [ ] **Step 4: Commit** (gộp với Task 3 nếu build chung)

```powershell
git add MORT/Service/Tts/TtsService.cs
git commit -m "feat(tts): add TtsService (WinRT speech, sequential queue)"
```

---

## Task 3: Settings TTS trong `AdvencedOptionManager`

**Files:**
- Modify: `MORT/AdvencedOptionManager.cs`

- [ ] **Step 1: Thêm const key** — sau dòng `public const string KeyMemoryRemainTime = "@MemoryRemainTime ";` (≈ dòng 157)

```csharp
        //Thuyết minh (TTS)
        public const string KeyTtsMode = "@TTS_MODE ";          //0 = ReadOriginal, 1 = ReadTranslated
        public const string KeyTtsVoiceName = "@TTS_VOICE_NAME ";
        public const string KeyTtsRate = "@TTS_RATE ";          //-10..+10
        public const string KeyTtsVolume = "@TTS_VOLUME ";      //0..100
```

- [ ] **Step 2: Thêm field vào class `Data`** — sau `public ISettingData<int> TranslateMemoryRemainTime;` (≈ dòng 246)

```csharp
            //Thuyết minh (TTS)
            public ISettingData<int> TtsMode;
            public ISettingData<string> TtsVoiceName;
            public ISettingData<int> TtsRate;
            public ISettingData<int> TtsVolume;
```

- [ ] **Step 3: Thêm getter + mutator** — sau `public static int TranslateMemoryRemainTime => data.TranslateMemoryRemainTime.Value;` (≈ dòng 428)

```csharp
        //Thuyết minh (TTS)
        public static int TtsMode => data.TtsMode.Value;
        public static string TtsVoiceName => data.TtsVoiceName.Value;
        public static int TtsRate => data.TtsRate.Value;
        public static int TtsVolume => data.TtsVolume.Value;

        public static void SetTts(int mode, string voiceName, int rate, int volume)
        {
            data.TtsMode.Value = mode;
            data.TtsVoiceName.Value = voiceName;
            data.TtsRate.Value = rate;
            data.TtsVolume.Value = volume;
        }
```

- [ ] **Step 4: Thêm `LoadTtsSetting()`** — đặt cạnh `LoadClipboardSaveSetting()` (≈ dòng 704)

```csharp
        private static void LoadTtsSetting()
        {
            data.TtsMode = SettingDataFactory.Create<int>(KeyTtsMode, data.ParseList, 1);
            data.TtsVoiceName = SettingDataFactory.Create<string>(KeyTtsVoiceName, data.ParseList, "");
            data.TtsRate = SettingDataFactory.Create<int>(KeyTtsRate, data.ParseList, 0);
            data.TtsVolume = SettingDataFactory.Create<int>(KeyTtsVolume, data.ParseList, 100);
        }
```

- [ ] **Step 5: Gọi trong `Init()`** — sau `LoadClipboardSaveSetting();` (≈ dòng 592)

```csharp
            LoadClipboardSaveSetting();

            //Thuyết minh (TTS)
            LoadTtsSetting();
```

- [ ] **Step 6: Verify compile** (build chung với Task 2)

Run: `dotnet build MORT/MORT.csproj -c Debug`
Expected: BUILD SUCCEEDED.

- [ ] **Step 7: Commit**

```powershell
git add MORT/AdvencedOptionManager.cs
git commit -m "feat(tts): persist TTS mode/voice/rate/volume in AdvencedOptionManager"
```

---

## Task 4: Cắm `TtsService` vào pipeline (DI + ProcessTranslateService + Form1)

**Files:**
- Modify: `MORT/Program.cs`
- Modify: `MORT/Service/ProcessTranslateService/ProcessTranslateService.cs`
- Modify: `MORT/Form1.cs`

- [ ] **Step 1: Đăng ký DI** — `MORT/Program.cs` `ConfigureServices`, sau `services.AddSingleton<CustomApiPresetService>();` (≈ dòng 242)

```csharp
            services.AddSingleton<CustomApiPresetService>();
            services.AddSingleton<MORT.Service.Tts.TtsService>();
```

- [ ] **Step 2: Thêm field + using** vào `ProcessTranslateService.cs`

Thêm using ở đầu file: `using MORT.Service.Tts;`

Sau `private readonly bool _isAvailableWinOCR;` (≈ dòng 55):

```csharp
        private readonly TtsService _ttsService;
        private string _lastSpoken = ""; //câu vừa đọc (cho re-read)
```

- [ ] **Step 3: Mở rộng constructor** — thay signature + thêm gán

Tìm (≈ dòng 66):

```csharp
        public ProcessTranslateService(Form parent, TranslateResultMemoryService memoryService, SettingManager settingManager, WindowOcr loader, bool isAvailableWinOCR, Action<bool> OnStopTranslate)
        {
            _parent = parent;
            _memoryService = memoryService;
            _settingManager = settingManager;
            _winOcr = loader;
            _oneOcr = new OneOcr();
            _isAvailableWinOCR = isAvailableWinOCR;
            this.OnStopTranslate = OnStopTranslate;
        }
```

Thay bằng:

```csharp
        public ProcessTranslateService(Form parent, TranslateResultMemoryService memoryService, SettingManager settingManager, WindowOcr loader, bool isAvailableWinOCR, Action<bool> OnStopTranslate, TtsService ttsService)
        {
            _parent = parent;
            _memoryService = memoryService;
            _settingManager = settingManager;
            _winOcr = loader;
            _oneOcr = new OneOcr();
            _isAvailableWinOCR = isAvailableWinOCR;
            this.OnStopTranslate = OnStopTranslate;
            _ttsService = ttsService;
        }
```

- [ ] **Step 4: Repoint `DoTextToSpeach`** (≈ dòng 436-448)

Tìm:

```csharp
        public void DoTextToSpeach(string text)
        {
            if (_isAvailableWinOCR && _settingManager.IsUseTTS)
            {
                int type = 0;
                if (_settingManager.IsWaitTTSEnd)
                {
                    type = 1;
                }

                _winOcr.TextToSpeach(text, type);
            }
        }
```

Thay bằng:

```csharp
        public void DoTextToSpeach(string text)
        {
            if (_isAvailableWinOCR && _settingManager.IsUseTTS)
            {
                _ttsService.Speak(text); //hàng đợi tuần tự thay cho IsWaitTTSEnd cũ
            }
        }

        /// <summary>Đọc lại câu vừa thuyết minh — dùng cho hotkey, chạy cả khi auto-read tắt.</summary>
        public void RereadLast()
        {
            if (_isAvailableWinOCR && !string.IsNullOrWhiteSpace(_lastSpoken))
            {
                _ttsService.SpeakNow(_lastSpoken);
            }
        }
```

- [ ] **Step 5: Mode branch tại khối dispatch** (≈ dòng 970-979)

Tìm:

```csharp
                                //TTS 처리
                                if (_settingManager.NowSkin == SettingManager.Skin.over)
                                {
                                    string transResult = finalTransResult.Replace(Util.GetSpliteToken(TransType), "", StringComparison.InvariantCulture);
                                    DoTextToSpeach(transResult);
                                }
                                else
                                {
                                    DoTextToSpeach(finalTransResult);
                                }
```

Thay bằng:

```csharp
                                //TTS 처리 (thuyết minh) — chọn nguồn theo chế độ ReadOriginal/ReadTranslated
                                string ttsSource =
                                    (MORT.Service.Tts.TtsReadMode)AdvencedOptionManager.TtsMode == MORT.Service.Tts.TtsReadMode.ReadOriginal
                                        ? NowOcrString
                                        : finalTransResult;

                                if (_settingManager.NowSkin == SettingManager.Skin.over)
                                {
                                    ttsSource = ttsSource.Replace(Util.GetSpliteToken(TransType), "", StringComparison.InvariantCulture);
                                }

                                _lastSpoken = ttsSource;
                                DoTextToSpeach(ttsSource);
```

- [ ] **Step 6: Inject vào Form1 + truyền xuống** — `MORT/Form1.cs`

Thêm using: `using MORT.Service.Tts;`

Thêm field (gần các service field khác, ≈ dòng 305):

```csharp
        private readonly TtsService _ttsService;
```

Sửa constructor (≈ dòng 705):

```csharp
        public Form1(GeminiConfigMaker geminiConfigMaker, TranslateTypListService translateTypListService, TtsService ttsService)
        {
            _translateTypListService = translateTypListService;
            _ttsService = ttsService;
```

Sửa dòng tạo `ProcessTranslateService` (≈ dòng 816):

```csharp
                _processTranslateService = new ProcessTranslateService(this, _translateResultMemoryService, MySettingManager, loader, isAvailableWinOCR, StopTrans, _ttsService);
```

- [ ] **Step 7: Verify compile**

Run: `dotnet build MORT/MORT.csproj -c Debug`
Expected: BUILD SUCCEEDED.

- [ ] **Step 8: Verify regression test vẫn pass**

Run: `dotnet test MORT.Tests/MORT.Tests.csproj`
Expected: PASS (6/6).

- [ ] **Step 9: Commit**

```powershell
git add MORT/Program.cs MORT/Service/ProcessTranslateService/ProcessTranslateService.cs MORT/Form1.cs
git commit -m "feat(tts): wire TtsService into translate pipeline with read-mode branch"
```

---

## Task 5: Hotkey (enum + dispatch trong Form1)

**Files:**
- Modify: `MORT/KeyInputLabel.cs`
- Modify: `MORT/Form1.cs`

- [ ] **Step 1: Thêm `KeyType` ở CUỐI enum** (≈ dòng 9-14) — KHÔNG đổi thứ tự cũ

Tìm:

```csharp
        public enum KeyType
        {
            None, Translate, OpenDic, QuickOCR, SnapShot, TranslateOnce, Hide,
            OpenSetting, LayerTransparency, DBTranslate, NaverTranslate, GoogleTranslate, GoogleSheetTranslate, EzTrans, DeepL, PapagoWeb,
            OcrAreaFollowMouse
        }
```

Thay bằng:

```csharp
        public enum KeyType
        {
            None, Translate, OpenDic, QuickOCR, SnapShot, TranslateOnce, Hide,
            OpenSetting, LayerTransparency, DBTranslate, NaverTranslate, GoogleTranslate, GoogleSheetTranslate, EzTrans, DeepL, PapagoWeb,
            OcrAreaFollowMouse,
            ToggleTtsAutoRead, RereadTts, ToggleTtsMode
        }
```

- [ ] **Step 2: Thêm 3 case dispatch** trong `gHook_KeyDown` — sau `case KeyInputLabel.KeyType.PapagoWeb:` ... `break;` (≈ dòng 1284, trước `}` đóng switch ở 1285)

```csharp
                        case KeyInputLabel.KeyType.ToggleTtsAutoRead:
                            MySettingManager.IsUseTTS = !MySettingManager.IsUseTTS;
                            FormManager.Instace.AddText(MySettingManager.IsUseTTS ? "TTS: ON" : "TTS: OFF");
                            break;
                        case KeyInputLabel.KeyType.RereadTts:
                            _processTranslateService.RereadLast();
                            break;
                        case KeyInputLabel.KeyType.ToggleTtsMode:
                            {
                                int newMode = AdvencedOptionManager.TtsMode == 0 ? 1 : 0;
                                AdvencedOptionManager.SetTts(newMode, AdvencedOptionManager.TtsVoiceName, AdvencedOptionManager.TtsRate, AdvencedOptionManager.TtsVolume);
                                AdvencedOptionManager.Save();
                                FormManager.Instace.AddText(newMode == 0 ? "TTS: đọc gốc" : "TTS: đọc bản dịch");
                            }
                            break;
```

> Field đúng tên là `_processTranslateService` (xem dòng 816). KHÔNG nhầm với `_processTrans` (Thread).

- [ ] **Step 3: Verify compile**

Run: `dotnet build MORT/MORT.csproj -c Debug`
Expected: BUILD SUCCEEDED.

- [ ] **Step 4: Commit**

```powershell
git add MORT/KeyInputLabel.cs MORT/Form1.cs
git commit -m "feat(tts): add toggle/reread/mode hotkey dispatch"
```

---

## Task 6: UI cấu hình TTS trong `UIAdvencedOption` (+ Designer)

**Files:**
- Modify: `MORT/UIAdvencedOption.cs`
- Modify: `MORT/UIAdvencedOption.Designer.cs` (qua WinForms Designer)

> Công tắc bật/tắt (`IsUseTTS`) đã có sẵn ở `FormOption.cs` (cbUseTTS) — **không trùng lặp** ở đây. Tab này chỉ thêm: chế độ, giọng, tốc độ, âm lượng + 3 ô hotkey.

- [ ] **Step 1: Thêm control trên Designer** (mở `UIAdvencedOption` trong WinForms Designer, kéo thả vào 1 group/tab mới "Thuyết minh")

Đặt đúng tên (code bên dưới phụ thuộc tên này):
- `ComboBox cbTtsMode` — 2 item: "Đọc bản gốc", "Đọc bản dịch".
- `ComboBox cbTtsVoice` — DropDownList.
- `Label lbTtsVoiceWarning` — ẩn mặc định; báo khi thiếu giọng vi.
- `TrackBar tbTtsRate` — Min=-10, Max=10, TickFrequency=5.
- `TrackBar tbTtsVolume` — Min=0, Max=100, TickFrequency=10.
- 3 control `CustomControl.CtHotKey`: `ctTtsToggle`, `ctTtsReread`, `ctTtsMode`.

- [ ] **Step 2: Đăng ký 3 hotkey** trong `Init()` — sau dòng `AddHotKey(... KeyInputLabel.KeyType.PapagoWeb, ctPapagoWeb);` (≈ dòng 68)

```csharp
            AddHotKey(LocalizeString("Adv HotKey TtsToggle"), LocalizeString("Adv HotKey TtsToggle Info"), KeyInputLabel.KeyType.ToggleTtsAutoRead, ctTtsToggle);
            AddHotKey(LocalizeString("Adv HotKey TtsReread"), LocalizeString("Adv HotKey TtsReread Info"), KeyInputLabel.KeyType.RereadTts, ctTtsReread);
            AddHotKey(LocalizeString("Adv HotKey TtsMode"), LocalizeString("Adv HotKey TtsMode Info"), KeyInputLabel.KeyType.ToggleTtsMode, ctTtsMode);
```

> Hotkey tự lưu qua `SetHotKey()` (lặp `hotKeyDic`) và tự load qua `InitData()` — **không cần sửa thêm**. Localize string thiếu sẽ hiện rỗng, không lỗi; bổ sung vào `Resources/localize.csv` nếu muốn nhãn đẹp.

- [ ] **Step 3: Load giá trị vào control** — thêm vào cuối `InitData()` (sau khối load khác, trước khi method kết thúc; `_isInit` đang `false` nên set an toàn)

```csharp
            //Thuyết minh (TTS)
            cbTtsMode.SelectedIndex = (AdvencedOptionManager.TtsMode == 0) ? 0 : 1;
            tbTtsRate.Value = Math.Max(tbTtsRate.Minimum, Math.Min(tbTtsRate.Maximum, AdvencedOptionManager.TtsRate));
            tbTtsVolume.Value = Math.Max(tbTtsVolume.Minimum, Math.Min(tbTtsVolume.Maximum, AdvencedOptionManager.TtsVolume));

            var ttsService = Program.ServiceContainer?.GetService(typeof(MORT.Service.Tts.TtsService)) as MORT.Service.Tts.TtsService;
            cbTtsVoice.Items.Clear();
            if (ttsService != null && ttsService.HasVietnameseVoice)
            {
                lbTtsVoiceWarning.Visible = false;
                foreach (var name in ttsService.GetVietnameseVoiceNames())
                {
                    cbTtsVoice.Items.Add(name);
                }
                int idx = cbTtsVoice.Items.IndexOf(AdvencedOptionManager.TtsVoiceName);
                cbTtsVoice.SelectedIndex = (idx >= 0) ? idx : (cbTtsVoice.Items.Count > 0 ? 0 : -1);
            }
            else
            {
                lbTtsVoiceWarning.Text = "Chưa cài giọng tiếng Việt. Vào Windows Settings > Time & Language > Language > thêm Tiếng Việt (bật Text-to-speech).";
                lbTtsVoiceWarning.Visible = true;
            }
```

Thêm `using System;` nếu file chưa có (kiểm tra đầu file; thường đã có).

- [ ] **Step 4: Lưu giá trị** — thêm method `SetTtsSetting()` (đặt trong region phù hợp)

```csharp
        private void SetTtsSetting()
        {
            int mode = (cbTtsMode.SelectedIndex == 0) ? 0 : 1;
            string voice = cbTtsVoice.SelectedItem?.ToString() ?? "";
            AdvencedOptionManager.SetTts(mode, voice, tbTtsRate.Value, tbTtsVolume.Value);
        }
```

- [ ] **Step 5: Gọi khi Apply** — trong `OnClickApply` (≈ dòng 661), thêm trước `AdvencedOptionManager.Save();`

```csharp
            SetAppLanguage();
            SetTtsSetting();

            AdvencedOptionManager.Save();
```

- [ ] **Step 6: Verify compile**

Run: `dotnet build MORT/MORT.csproj -c Debug`
Expected: BUILD SUCCEEDED.

- [ ] **Step 7: Commit**

```powershell
git add MORT/UIAdvencedOption.cs MORT/UIAdvencedOption.Designer.cs MORT/UIAdvencedOption.resx
git commit -m "feat(tts): add TTS config tab (mode/voice/rate/volume + hotkeys)"
```

---

## Task 7: Verify end-to-end (thủ công) + hoàn tất

**Files:** (không sửa code)

- [ ] **Step 1: Build bản chạy được**

Mở `MORT.sln` trong Visual Studio 2022 (x64), đảm bảo có `MORT_CORE.DLL`/`nhocr.DLL` (theo README MORT), Build → Run. Hoặc dùng bản release giải nén rồi thay binary đã build.

- [ ] **Step 2: Kịch bản kiểm thử**

Bật game/đoạn chữ tiếng Việt, vẽ vùng OCR. Kiểm:
1. Vào Options bật TTS (cbUseTTS) → chọn giọng Việt ở tab Thuyết minh → Apply.
2. **Đọc bản dịch:** mode=Đọc bản dịch → có câu mới → nghe đọc tiếng Việt bản dịch.
3. **Đọc gốc:** đổi mode (hoặc hotkey ToggleTtsMode) → nghe đọc thẳng text gốc, không qua dịch.
4. **Chống trùng:** để yên màn hình → KHÔNG đọc lặp lại cùng câu mỗi ~2s.
5. **Tuần tự:** thoại đổi nhanh → đọc hết câu rồi mới câu kế, không chồng tiếng; backlog không vượt 3.
6. **Hotkey:** ToggleTtsAutoRead bật/tắt; RereadTts đọc lại câu hiện tại **kể cả khi auto tắt**.
7. **Tốc độ/âm lượng:** chỉnh trackbar → Apply → nghe đổi.
8. **Thiếu giọng vi:** (máy chưa cài) → tab Thuyết minh hiện cảnh báo, app không crash.

- [ ] **Step 3: Gỡ smoke test tạm** (nếu Task 2 Step 3 còn dòng test trong Form1) — đảm bảo đã xoá.

- [ ] **Step 4: Cập nhật trạng thái spec**

Sửa header spec `Trạng thái: Approved (chờ review spec)` → `Implemented`.

- [ ] **Step 5: Commit cuối + (tuỳ chọn) mở PR**

```powershell
git add -A
git commit -m "docs(tts): mark spec implemented after e2e verification"
```

Dùng skill `superpowers:finishing-a-development-branch` để chọn merge/PR.

---

## Self-Review

**1. Spec coverage:**
- 2 chế độ XOR → Task 1 (enum) + Task 4 Step 5 (branch) ✔
- Giọng Việt (cả 2 mode) → Task 2 `PickVoice` ✔
- Auto-read câu mới → Task 4 (khối dispatch dedup) ✔
- Hotkey toggle/re-read/mode → Task 5 ✔
- Đọc tuần tự + cap 3 + chống trùng → Task 1 `TtsSpeechQueue` + Task 2 queue pump ✔
- Settings mode/voice/rate/volume → Task 3 ✔; UI → Task 6 ✔
- Reuse `IsUseTTS` làm enable → Task 4 Step 4 (gate) + FormOption sẵn có ✔
- Bỏ Tesseract v1 → không hook nhánh Tesseract (chỉ khối dispatch chung) ✔
- Cảnh báo thiếu giọng vi → Task 6 Step 3 ✔
- Không đổi `.csproj` → đúng (TFM đã có WinRT) ✔

**2. Placeholder scan:** Không có TBD/TODO trong code mới. UI Designer mô tả rõ tên control + code phụ thuộc. ✔

**3. Type consistency:**
- `TtsReadMode { ReadOriginal=0, ReadTranslated=1 }` dùng nhất quán (Task 1, 4).
- `TtsService.Speak/SpeakNow/Stop/HasVietnameseVoice/GetVietnameseVoiceNames/RefreshVoices` — định nghĩa Task 2, dùng Task 4/6 khớp.
- `AdvencedOptionManager.TtsMode/TtsVoiceName/TtsRate/TtsVolume/SetTts(int,string,int,int)` — Task 3 định nghĩa, Task 2/4/5/6 dùng khớp.
- `ProcessTranslateService.RereadLast()` — Task 4 định nghĩa, Task 5 dùng. `_processTranslateService` đúng field.
- `TtsSpeechQueue.Enqueue/TryDequeue/Count/Clear/ResetDedup/MaxSize` — Task 1 định nghĩa, Task 2 dùng khớp.

**Lưu ý rủi ro còn lại (đã ghi trong spec):** dual settings store (enable ở legacy, còn lại ở modern); Tesseract không phủ; giọng mới cài cần `RefreshVoices()`/restart để thấy.
