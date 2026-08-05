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
};

export type Voice = { label: string; id: string };

/// Một mục log — giữ cả bản gốc để biết nghe sai là do OCR hay do dịch.
export type Spoken = { src: string; out: string };

export const MODES: { value: string; label: string }[] = [
  { value: 'en', label: 'Đọc bản dịch (Anh→Việt)' },
  { value: 'zh', label: 'Đọc bản dịch (Trung→Việt)' },
];

// Đo trên 6 mẫu khó: OneOCR 0.991 / Windows 0.625. Đổi lại OneOCR ~70ms/khung vs ~20ms
// và cần Win11 + Snipping Tool; không có thì tự lùi về Windows OCR (báo ở thanh trạng thái).
export const ENGINES: { value: string; label: string }[] = [
  { value: 'oneocr', label: 'OneOCR — chính xác hơn (cần Win11)' },
  { value: 'windows', label: 'Windows OCR — nhẹ, nhanh hơn' },
];

// Đo lúc CPU bị tranh (giả lập game kín 6 nhân), cả hai đều là tiến trình riêng ở
// BELOW_NORMAL — xem README mục "Giọng đọc".
export const TTS_ENGINES: { value: string; label: string; ghi_chu: string }[] = [
  { value: 'windows', label: 'Giọng Windows — có sẵn', ghi_chu: '0 MB, cài xong chạy ngay' },
  {
    value: 'piper',
    label: 'Piper — neural, nhẹ',
    ghi_chu: 'tải ~82 MB · 97 MB RAM · chữ tiếng Anh đọc bằng âm Anh nhưng giọng chỉ học tiếng Việt',
  },
  {
    value: 'vieneu',
    label: 'VieNeu — neural 48 kHz, đọc lẫn Anh–Việt',
    ghi_chu: 'tải ~560 MB · 1,3 GB RAM · êm hơn Piper khi chơi game, đổi lại nặng hơn nhiều',
  },
];

// 14 giọng có sẵn của VieNeu v3 Turbo (đọc từ chính model lúc chạy thử).
export const VIENEU_VOICES: string[] = [
  'Minh Đức', 'Phạm Tuyên', 'Thái Sơn', 'Xuân Vĩnh', 'Thanh Bình', 'Trúc Ly', 'Ngọc Linh',
  'Đoan Trang', 'Mai Anh', 'Thục Đoan', 'Minh Triết', 'Thùy Dung', 'Quang Sơn', 'Ngọc Trân',
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
