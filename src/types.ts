// Khớp 1-1 với config.rs / tts.rs (serde giữ nguyên snake_case).
export type Config = {
  asr_engine: string;
  speechmatics_key: string;
  asr_language: string;
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

/// Một mục log — giữ cả bản gốc để biết nghe sai hay dịch sai.
export type Spoken = { src: string; out: string };

// Nghe bằng gì. Bản chạy máy đo được RTF 0,055; Speechmatics đẩy tiếng lên mạng nên CPU gần 0
// nhưng lời thoại rời khỏi máy.
export const ASR_ENGINES: { value: string; label: string; ghi_chu: string }[] = [
  {
    value: 'local',
    label: 'Chạy trên máy — miễn phí',
    ghi_chu: 'không gửi gì ra ngoài · ~5,5% một nhân',
  },
  {
    value: 'speechmatics',
    label: 'Speechmatics — cần API key',
    ghi_chu: 'nghe chuẩn hơn khi nhạc nền to · lời thoại được gửi lên máy chủ của họ',
  },
];

// Tiếng trong game. Bản chạy máy: tiếng Anh dùng zipformer, còn lại dùng SenseVoice (asr.rs).
export const ASR_LANGS: { value: string; label: string }[] = [
  { value: 'en', label: 'Tiếng Anh' },
  { value: 'zh', label: 'Tiếng Trung' },
  { value: 'ja', label: 'Tiếng Nhật' },
  { value: 'ko', label: 'Tiếng Hàn' },
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
