// Khớp 1-1 với config.rs / tts.rs (serde giữ nguyên snake_case).
export type Region = { left: number; top: number; width: number; height: number };

export type Config = {
  region: Region;
  exclude_region: Region | null;
  interval_ms: number;
  dedup_require_stable: boolean;
  min_line_len: number;
  fuzzy_threshold: number;
  mode: string;
  source: string;
  asr_engine: string;
  speechmatics_key: string;
  asr_language: string;
  ocr_engine: string;
  ocr_language: string;
  min_clean_ratio: number;
  translate: boolean;
  translate_from: string;
  translate_to: string;
  translate_engine: string;
  llm_base_url: string;
  llm_model: string;
  llm_api_key: string;
  llm_context_lines: number;
  tts_engine: string;
  voice_name: string;
  speed: number;
  volume: number;
  captions: boolean;
  captions_movable: boolean;
};

export type Voice = { label: string; id: string };

/// Một mục log — giữ cả bản gốc để biết nghe sai là do OCR hay do dịch.
export type Spoken = { src: string; out: string };

export const MODES: { value: string; label: string }[] = [
  { value: 'en', label: 'Đọc bản dịch (Anh→Việt)' },
  { value: 'zh', label: 'Đọc bản dịch (Trung→Việt)' },
];

// Lấy lời thoại từ đâu. Nghe tiếng đo được RTF 0,055 — khoảng 5,5% một nhân, chạy cùng game
// thoải mái. Bản chạy máy chỉ biết tiếng Anh nên chế độ Trung→Việt phải dùng màn hình, hoặc
// chuyển sang Speechmatics.
export const SOURCES: { value: string; label: string; ghi_chu: string }[] = [
  {
    value: 'screen',
    label: 'Màn hình — đọc phụ đề (OCR)',
    ghi_chu: 'cần game có phụ đề và phải chọn đúng vùng · chính xác hơn khi phụ đề rõ',
  },
  {
    value: 'audio',
    label: 'Âm thanh — nghe thoại game',
    ghi_chu: 'dùng khi game không có phụ đề · nhạc/tiếng nổ to sẽ nghe sai',
  },
];

// Nghe bằng gì. Bản chạy máy đo được RTF 0,055; Speechmatics đẩy tiếng lên mạng nên CPU gần 0
// nhưng lời thoại rời khỏi máy.
export const ASR_ENGINES: { value: string; label: string; ghi_chu: string }[] = [
  {
    value: 'local',
    label: 'Chạy trên máy — miễn phí',
    ghi_chu: 'tải 198 MB · CHỈ tiếng Anh · không gửi gì ra ngoài · ~5,5% một nhân',
  },
  {
    value: 'speechmatics',
    label: 'Speechmatics — cần API key',
    ghi_chu: 'nghe chuẩn hơn khi nhạc nền to · nhiều thứ tiếng · LỜI THOẠI ĐƯỢC GỬI LÊN MÁY CHỦ HỌ',
  },
];

// Mã ngôn ngữ CHỈ dành cho Speechmatics. Bản chạy máy bỏ qua, model của nó chỉ có tiếng Anh.
export const ASR_LANGS: { value: string; label: string }[] = [
  { value: 'en', label: 'Tiếng Anh' },
  { value: 'zh', label: 'Tiếng Trung' },
  { value: 'ja', label: 'Tiếng Nhật' },
  { value: 'ko', label: 'Tiếng Hàn' },
];

// Đo trên 6 mẫu khó: OneOCR 0.991 / Windows 0.625. Đổi lại OneOCR ~70ms/khung vs ~20ms
// và cần Win11 + Snipping Tool; không có thì tự lùi về Windows OCR (báo ở thanh trạng thái).
export const ENGINES: { value: string; label: string }[] = [
  { value: 'oneocr', label: 'OneOCR — chính xác hơn (cần Win11)' },
  { value: 'windows', label: 'Windows OCR — nhẹ, nhanh hơn' },
];

// Đo lúc CPU bị tranh (giả lập game kín 6 nhân), VieNeu chạy tiến trình riêng ở BELOW_NORMAL
// — xem README mục "Giọng đọc".
export const TTS_ENGINES: { value: string; label: string; ghi_chu: string }[] = [
  {
    value: 'windows',
    label: 'Giọng Windows — có sẵn',
    ghi_chu: '0 MB, cài xong chạy ngay · cũng là giọng dự phòng khi VieNeu trục trặc',
  },
  {
    value: 'vieneu',
    label: 'VieNeu — neural 48 kHz, đọc lẫn Anh–Việt',
    ghi_chu: 'tải ~560 MB · 1,3 GB RAM · đọc chữ tiếng Anh tốt, gần như không làm giật game',
  },
];

// Provider dùng chung API OpenAI (`/chat/completions`) nên chỉ khác base_url + model.
// Hạn mức free tier tra 8/2026 — Groq là cái duy nhất chịu nổi cả buổi chơi liên tục.
export const LLM_PRESETS: { name: string; base_url: string; model: string; free: string }[] = [
  {
    name: 'Groq',
    base_url: 'https://api.groq.com/openai/v1',
    model: 'llama-3.3-70b-versatile',
    free: '30 lệnh/phút · 14.400/ngày — lấy key ở console.groq.com',
  },
  {
    name: 'Gemini',
    base_url: 'https://generativelanguage.googleapis.com/v1beta/openai',
    model: 'gemini-2.5-flash-lite',
    free: 'dịch mượt nhất, nhưng free chỉ 15/phút · 1.000/ngày',
  },
  {
    name: 'OpenRouter',
    base_url: 'https://openrouter.ai/api/v1',
    model: 'meta-llama/llama-3.3-70b-instruct:free',
    free: 'model :free ~50 lệnh/ngày',
  },
];
