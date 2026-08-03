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
