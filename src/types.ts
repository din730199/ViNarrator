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
