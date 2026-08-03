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
  translate: boolean;
  translate_from: string;
  translate_to: string;
  voice_name: string;
  speed: number;
  volume: number;
};

export type Voice = { label: string; id: string };

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
